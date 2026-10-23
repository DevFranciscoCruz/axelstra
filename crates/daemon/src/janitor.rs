//! Removes the worktrees Axelstra's agents created (branches `ax/...`) once their work is merged: the branch is in
//! the default branch already, or GitHub says its pull request was merged (squash merges leave no ancestor). A
//! worktree with a running session, uncommitted changes or unmerged work is never touched.

use crate::projects::{self, Project};
use anyhow::Result;
use axelstra_core::git;
use serde_json::{json, Value};
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

/// Longest a network call (`git fetch`, `gh`) of the sweep may take; the scheduler loop must not wait on a hung one.
const NET_TIMEOUT: Duration = Duration::from_secs(30);

/// Runs `cmd` and kills it after `limit`; `None` when it cannot start or ran out of time.
fn output_within(cmd: &mut Command, limit: Duration) -> Option<Output> {
    let mut child = cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().ok()?;
    let (out, err) = (child.stdout.take()?, child.stderr.take()?);
    let drain = |mut r: Box<dyn std::io::Read + Send>| {
        std::thread::spawn(move || {
            let mut b = Vec::new();
            let _ = r.read_to_end(&mut b);
            b
        })
    };
    let (ot, et) = (drain(Box::new(out)), drain(Box::new(err)));
    let deadline = Instant::now() + limit;
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    };
    Some(Output { status, stdout: ot.join().unwrap_or_default(), stderr: et.join().unwrap_or_default() })
}

/// Why this branch counts as merged, `None` when it does not (or cannot be told).
pub fn merged_reason(repo: &Path, branch: &str, default: &str) -> Option<String> {
    // a branch nobody committed to is "inside" the default branch too, and must not be mistaken for merged work
    if !has_own_commits(repo, branch) {
        return None;
    }
    let tip = git::run(repo, &["rev-parse", branch]).ok().filter(|o| o.ok).map(|o| o.stdout.trim().to_string())?;
    let upstream = format!("origin/{default}");
    if git::run(repo, &["merge-base", "--is-ancestor", branch, &upstream]).map(|o| o.ok).unwrap_or(false) {
        return Some(format!("already in {upstream}"));
    }
    let o = output_within(
        git::quiet(&mut Command::new("gh")).current_dir(repo).env("GH_PROMPT_DISABLED", "1").args([
            "pr",
            "list",
            "--state",
            "merged",
            "--head",
            branch,
            "--limit",
            "1",
            "--json",
            "number,headRefOid",
        ]),
        NET_TIMEOUT,
    )
    .filter(|o| o.status.success())?;
    let v: Value = serde_json::from_slice(&o.stdout).ok()?;
    let pr = v.as_array()?.first()?.clone();
    // the merged PR must be exactly what is here: commits added after the merge would be lost with the branch
    if pr["headRefOid"].as_str() != Some(tip.as_str()) {
        return None;
    }
    pr["number"].as_u64().map(|n| format!("PR #{n} merged"))
}

/// True when the branch's reflog records a commit made on it (a fresh branch only has its creation).
pub fn has_own_commits(repo: &Path, branch: &str) -> bool {
    git::run(repo, &["reflog", "show", "--format=%gs", branch])
        .map(|o| o.ok && o.stdout.lines().any(|l| l.starts_with("commit")))
        .unwrap_or(false)
}

fn sweep_project(p: &Project, busy: &dyn Fn(&str) -> bool, dry: bool, out: &mut Vec<Value>) -> Result<()> {
    let repo = Path::new(&p.path);
    let default = git::default_branch(repo);
    let mut fetched = false;
    for w in git::worktree_list(repo)? {
        let Some(branch) = w.branch.clone().filter(|b| b.starts_with("ax/")) else { continue };
        if w.is_main || busy(&w.path) || git::has_changes(Path::new(&w.path)).unwrap_or(true) {
            continue;
        }
        if !fetched {
            // so a merge done on GitHub since the last fetch is seen; a failure (offline) just means less is found
            let _ = output_within(
                git::quiet(&mut Command::new("git")).current_dir(repo).env("GIT_TERMINAL_PROMPT", "0").args(["fetch", "--quiet", "origin"]),
                NET_TIMEOUT,
            );
            fetched = true;
        }
        let Some(why) = merged_reason(repo, &branch, &default) else { continue };
        let removed = dry || projects::worktree_remove(p, &w.path).is_ok();
        out.push(json!({"project": p.id, "path": w.path, "branch": branch, "why": why, "removed": removed && !dry, "dry_run": dry}));
    }
    Ok(())
}

/// Sweeps every project; `busy(path)` says whether a live session works in that worktree.
pub fn sweep(store: &Path, busy: &dyn Fn(&str) -> bool, dry: bool) -> Vec<Value> {
    let mut out = vec![];
    for p in projects::load(store) {
        let _ = sweep_project(&p, busy, dry, &mut out);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hung_command_is_killed_at_the_limit() {
        let (prog, args): (&str, &[&str]) = if cfg!(windows) { ("ping", &["-n", "30", "127.0.0.1"]) } else { ("sleep", &["30"]) };
        let t = Instant::now();
        assert!(output_within(Command::new(prog).args(args), Duration::from_millis(300)).is_none());
        assert!(t.elapsed() < Duration::from_secs(10));
        let (prog, args): (&str, &[&str]) = if cfg!(windows) { ("cmd", &["/C", "echo", "hi"]) } else { ("echo", &["hi"]) };
        let o = output_within(Command::new(prog).args(args), Duration::from_secs(10)).unwrap();
        assert!(o.status.success() && String::from_utf8_lossy(&o.stdout).contains("hi"));
    }

    #[test]
    fn a_branch_inside_the_default_branch_is_merged_and_a_new_one_is_not() {
        let dir = std::env::temp_dir().join(format!("axl-jan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let g = |args: &[&str]| assert!(git::run(&dir, args).unwrap().ok, "git {args:?}");
        let commit = ["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "--allow-empty", "-m", "x"];
        g(&["init", "-q", "-b", "main"]);
        g(&commit);
        // a local "origin/main" stands in for the remote
        g(&["update-ref", "refs/remotes/origin/main", "HEAD"]);
        // created and never committed to: not merged work
        g(&["branch", "ax/fresh"]);
        assert!(merged_reason(&dir, "ax/fresh", "main").is_none());
        g(&["checkout", "-q", "-b", "ax/done"]);
        g(&commit);
        g(&["checkout", "-q", "main"]);
        g(&["merge", "-q", "--ff-only", "ax/done"]);
        g(&["update-ref", "refs/remotes/origin/main", "HEAD"]);
        g(&["checkout", "-q", "-b", "ax/new"]);
        g(&commit);
        assert_eq!(merged_reason(&dir, "ax/done", "main").as_deref(), Some("already in origin/main"));
        assert!(merged_reason(&dir, "ax/new", "main").is_none() || std::process::Command::new("gh").arg("--version").output().is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
