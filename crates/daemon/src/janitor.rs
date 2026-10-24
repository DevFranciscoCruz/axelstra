//! Removes the worktrees Axelstra's agents created (branches `ax/...`) once their work is merged: the branch is in
//! the default branch already, or GitHub says its pull request was merged (squash merges leave no ancestor). A
//! worktree with a running session, uncommitted changes or unmerged work is never touched.

use crate::projects::{self, Project};
use anyhow::Result;
use axelstra_core::exec::Captured;
use axelstra_core::git;
use serde_json::{json, Value};
use std::path::Path;
use std::process::Command;
use std::time::Duration;

/// Longest a network call (`git fetch`, `gh`) of the sweep may take; the scheduler loop must not wait on a hung one.
const NET_TIMEOUT: Duration = Duration::from_secs(30);

/// Output cap of the sweep's network calls.
const NET_MAX_OUT: usize = 1 << 20;

/// Runs `cmd` and kills its whole tree after `limit`; `None` when it cannot start or ran out of time.
fn output_within(cmd: &mut Command, limit: Duration) -> Option<Captured> {
    axelstra_core::exec::run_capped(cmd, limit, NET_MAX_OUT).ok().filter(|o| !o.timed_out)
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
    // merged into the local default branch but not pushed yet: the commits stay reachable from it, so removing the branch loses nothing
    if git::run(repo, &["merge-base", "--is-ancestor", branch, default]).map(|o| o.ok).unwrap_or(false) {
        return Some(format!("already in local {default}"));
    }
    let o = output_within(
        Command::new("gh").current_dir(repo).env("GH_PROMPT_DISABLED", "1").args([
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
    .filter(|o| o.success())?;
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
                Command::new("git").current_dir(repo).env("GIT_TERMINAL_PROMPT", "0").args(["fetch", "--quiet", "origin"]),
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
    use std::time::Instant;

    #[test]
    fn a_hung_command_is_killed_at_the_limit() {
        let (prog, args): (&str, &[&str]) = if cfg!(windows) { ("ping", &["-n", "30", "127.0.0.1"]) } else { ("sleep", &["30"]) };
        let t = Instant::now();
        assert!(output_within(Command::new(prog).args(args), Duration::from_millis(300)).is_none());
        assert!(t.elapsed() < Duration::from_secs(10));
        let (prog, args): (&str, &[&str]) = if cfg!(windows) { ("cmd", &["/C", "echo", "hi"]) } else { ("echo", &["hi"]) };
        let o = output_within(Command::new(prog).args(args), Duration::from_secs(10)).unwrap();
        assert!(o.success() && o.stdout_lossy().contains("hi"));
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
        // merged into main locally, origin/main still behind: also counts
        g(&["checkout", "-q", "main"]);
        g(&["checkout", "-q", "-b", "ax/local"]);
        g(&commit);
        g(&["checkout", "-q", "main"]);
        g(&["merge", "-q", "--ff-only", "ax/local"]);
        assert_eq!(merged_reason(&dir, "ax/local", "main").as_deref(), Some("already in local main"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sweep_removes_only_merged_idle_agent_worktrees() {
        let root = std::env::temp_dir().join(format!("axl-sweep-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let repo = root.join("proj");
        std::fs::create_dir_all(&repo).unwrap();
        let g = |dir: &Path, args: &[&str]| assert!(git::run(dir, args).unwrap().ok, "git {args:?}");
        let commit = ["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "--allow-empty", "-m", "x"];
        g(&repo, &["init", "-q", "-b", "main"]);
        g(&repo, &commit);
        let store = root.join("projects.json");
        let p = projects::add(&store, &repo.to_string_lossy()).unwrap();
        // done: committed on its branch, then merged into main
        let done = projects::worktree_create(&p, "done").unwrap();
        g(Path::new(&done.path), &commit);
        g(&repo, &["merge", "-q", "--ff-only", "ax/done"]);
        // busy: merged too, but a session works in it
        let busy = projects::worktree_create(&p, "busy").unwrap();
        g(Path::new(&busy.path), &commit);
        g(&repo, &["merge", "-q", "--ff-only", "ax/busy"]);
        // dirty: merged, with an uncommitted file
        let dirty = projects::worktree_create(&p, "dirty").unwrap();
        g(Path::new(&dirty.path), &commit);
        g(&repo, &["merge", "-q", "--ff-only", "ax/dirty"]);
        std::fs::write(Path::new(&dirty.path).join("wip.txt"), "w").unwrap();
        // open: unmerged work
        let open = projects::worktree_create(&p, "open").unwrap();
        g(Path::new(&open.path), &commit);
        // fresh: nobody committed anything
        let fresh = projects::worktree_create(&p, "fresh").unwrap();

        let is_busy = |path: &str| axelstra_core::paths::same(path, &busy.path);
        // a dry run reports and removes nothing
        let dry = sweep(&store, &is_busy, true);
        assert_eq!(dry.len(), 1, "{dry:?}");
        assert_eq!(dry[0]["branch"], "ax/done");
        assert_eq!(dry[0]["dry_run"], true);
        assert_eq!(dry[0]["removed"], false);
        assert!(Path::new(&done.path).exists());

        let real = sweep(&store, &is_busy, false);
        assert_eq!(real.len(), 1, "{real:?}");
        assert_eq!(real[0]["removed"], true);
        assert!(!Path::new(&done.path).exists());
        for kept in [&busy, &dirty, &open, &fresh] {
            assert!(Path::new(&kept.path).exists(), "{} must stay", kept.path);
        }
        let _ = std::fs::remove_dir_all(&root);
    }
}
