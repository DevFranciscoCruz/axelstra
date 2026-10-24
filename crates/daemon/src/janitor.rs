//! Removes the worktrees Axelstra's agents created (branches `ax/...`) once their work is integrated: the branch is in
//! the default branch already, it is inside another live `ax/*` branch (stacked integration), or GitHub says its pull
//! request was merged (squash merges leave no ancestor). A worktree with a running session that has not reported,
//! uncommitted changes or unmerged work is never touched, and neither is a branch whose commits no other ref holds.

use crate::projects::{self, Project};
use anyhow::Result;
use axelstra_core::exec::Captured;
use axelstra_core::git;
use serde_json::{json, Value};
use std::path::Path;
use std::process::Command;
use std::sync::Mutex;
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
    merged_reason_in(repo, branch, default, &[])
}

/// `merged_reason`, also counting the branch as integrated when its tip is inside one of `others` (the other live
/// `ax/*` branches): its commits stay reachable from there, so removing it loses nothing.
pub fn merged_reason_in(repo: &Path, branch: &str, default: &str, others: &[String]) -> Option<String> {
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
    for other in others.iter().filter(|o| o.as_str() != branch) {
        if git::run(repo, &["merge-base", "--is-ancestor", branch, other]).map(|o| o.ok).unwrap_or(false) {
            return Some(format!("already in {other}"));
        }
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

/// The branch a `why` from `merged_reason_in` says the work went into ("main", "ax/other"), for the notice.
fn integrated_into(why: &str, default: &str) -> String {
    match why.strip_prefix("already in ") {
        Some(rest) => rest.strip_prefix("local ").or_else(|| rest.strip_prefix("origin/")).unwrap_or(rest).to_string(),
        None => default.to_string(),
    }
}

static EVENTS: Mutex<Vec<Value>> = Mutex::new(Vec::new());

/// Takes (and clears) the worktrees the sweep removed since the last call, for the app's toast and notification center.
pub fn take_events() -> Vec<Value> {
    std::mem::take(&mut *EVENTS.lock().unwrap_or_else(|e| e.into_inner()))
}

fn push_event(e: Value) {
    let mut q = EVENTS.lock().unwrap_or_else(|e| e.into_inner());
    q.push(e);
    let n = q.len();
    if n > 100 {
        q.drain(..n - 100);
    }
}

fn sweep_project(p: &Project, busy: &dyn Fn(&str) -> bool, dry: bool, out: &mut Vec<Value>) -> Result<()> {
    let repo = Path::new(&p.path);
    let default = git::default_branch(repo);
    let mut fetched = false;
    // branches already removed by this pass cannot keep another one's commits alive
    let mut gone: Vec<String> = vec![];
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
        // a live `ax/*` branch is one that still exists in a worktree (a dry run removes none, so `gone` stays empty)
        let others: Vec<String> = git::worktree_list(repo)?
            .into_iter()
            .filter_map(|o| o.branch)
            .filter(|b| b.starts_with("ax/") && *b != branch && !gone.contains(b))
            .collect();
        let Some(why) = merged_reason_in(repo, &branch, &default, &others) else { continue };
        let removed = !dry && projects::worktree_remove(p, &w.path).is_ok();
        let into = integrated_into(&why, &default);
        if removed {
            gone.push(branch.clone());
            push_event(json!({"project": p.id, "path": w.path, "branch": branch, "into": into, "why": why, "at": now_ms()}));
        }
        out.push(json!({"project": p.id, "path": w.path, "branch": branch, "into": into, "why": why, "removed": removed, "dry_run": dry}));
    }
    Ok(())
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
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

    fn repo_with_store(tag: &str) -> (std::path::PathBuf, std::path::PathBuf, Project) {
        let root = std::env::temp_dir().join(format!("axl-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let repo = root.join("proj");
        std::fs::create_dir_all(&repo).unwrap();
        let g = |args: &[&str]| assert!(git::run(&repo, args).unwrap().ok, "git {args:?}");
        g(&["init", "-q", "-b", "main"]);
        g(&["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "--allow-empty", "-m", "x"]);
        let store = root.join("projects.json");
        let p = projects::add(&store, &repo.to_string_lossy()).unwrap();
        (root, store, p)
    }

    const COMMIT: [&str; 8] = ["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "--allow-empty", "-m"];

    fn commit_in(dir: &Path, msg: &str) {
        let mut a = COMMIT.to_vec();
        a.push(msg);
        assert!(git::run(dir, &a).unwrap().ok);
    }

    #[test]
    fn a_stacked_branch_inside_another_live_branch_is_swept_but_the_top_one_stays() {
        let (root, store, p) = repo_with_store("stack");
        let base = projects::worktree_create(&p, "base").unwrap();
        commit_in(Path::new(&base.path), "base work");
        // the top branch grows from the base one and adds its own, unmerged commit
        let top = projects::create_at(&p, "top", "ax/base", true).unwrap().0;
        commit_in(Path::new(&top.path), "top work");
        let out = sweep(&store, &|_| false, false);
        assert_eq!(out.len(), 1, "{out:?}");
        assert_eq!(out[0]["branch"], "ax/base");
        assert_eq!(out[0]["into"], "ax/top");
        assert_eq!(out[0]["removed"], true);
        assert!(!Path::new(&base.path).exists());
        assert!(Path::new(&top.path).exists(), "the top branch holds unmerged work");
        // its commits are still reachable: the base commit is in the top branch
        assert!(git::run(Path::new(&p.path), &["rev-parse", "--verify", "ax/top"]).unwrap().ok);
        let ev = take_events();
        assert!(ev.iter().any(|e| e["branch"] == "ax/base" && e["into"] == "ax/top"), "{ev:?}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn two_branches_at_the_same_commit_do_not_delete_each_other() {
        let (root, store, p) = repo_with_store("twins");
        let a = projects::worktree_create(&p, "a").unwrap();
        commit_in(Path::new(&a.path), "work");
        let b = projects::create_at(&p, "b", "ax/a", true).unwrap().0;
        // b has no commit of its own: same tip as a; only one of them may go, the work must stay on a branch
        let out = sweep(&store, &|_| false, false);
        let removed = out.iter().filter(|o| o["removed"] == true).count();
        assert!(removed <= 1, "{out:?}");
        let left = [&a, &b].iter().filter(|w| Path::new(&w.path).exists()).count();
        assert!(left >= 1);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_stacked_branch_is_kept_when_dirty_or_busy_or_when_the_other_branch_is_not_a_superset() {
        let (root, store, p) = repo_with_store("stackkeep");
        let base = projects::worktree_create(&p, "base").unwrap();
        commit_in(Path::new(&base.path), "base work");
        let top = projects::create_at(&p, "top", "ax/base", true).unwrap().0;
        commit_in(Path::new(&top.path), "top work");
        // unrelated branch with its own commit: not inside anything
        let solo = projects::worktree_create(&p, "solo").unwrap();
        commit_in(Path::new(&solo.path), "solo work");
        std::fs::write(Path::new(&base.path).join("wip.txt"), "w").unwrap();
        let out = sweep(&store, &|_| false, false);
        assert!(out.is_empty(), "dirty base and unmerged solo/top must stay: {out:?}");
        std::fs::remove_file(Path::new(&base.path).join("wip.txt")).unwrap();
        let busy_base = base.path.clone();
        let out = sweep(&store, &|path| axelstra_core::paths::same(path, &busy_base), false);
        assert!(out.is_empty(), "busy base must stay: {out:?}");
        for w in [&base, &top, &solo] {
            assert!(Path::new(&w.path).exists());
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn integrated_into_names_the_target() {
        assert_eq!(integrated_into("already in origin/main", "main"), "main");
        assert_eq!(integrated_into("already in local main", "main"), "main");
        assert_eq!(integrated_into("already in ax/top", "main"), "ax/top");
        assert_eq!(integrated_into("PR #4 merged", "main"), "main");
    }
}
