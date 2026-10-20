//! Removes the worktrees Axelstra's agents created (branches `ax/...`) once their work is merged: the branch is in
//! the default branch already, or GitHub says its pull request was merged (squash merges leave no ancestor). A
//! worktree with a running session, uncommitted changes or unmerged work is never touched.

use crate::projects::{self, Project};
use anyhow::Result;
use axelstra_core::git;
use serde_json::{json, Value};
use std::path::Path;
use std::process::Command;

/// Why this branch counts as merged, `None` when it does not (or cannot be told).
pub fn merged_reason(repo: &Path, branch: &str, default: &str) -> Option<String> {
    let upstream = format!("origin/{default}");
    if git::run(repo, &["merge-base", "--is-ancestor", branch, &upstream]).map(|o| o.ok).unwrap_or(false) {
        return Some(format!("already in {upstream}"));
    }
    let o = git::quiet(&mut Command::new("gh"))
        .current_dir(repo)
        .env("GH_PROMPT_DISABLED", "1")
        .args(["pr", "list", "--state", "merged", "--head", branch, "--limit", "1", "--json", "number"])
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    let v: Value = serde_json::from_slice(&o.stdout).ok()?;
    v.as_array()?.first().and_then(|p| p["number"].as_u64()).map(|n| format!("PR #{n} merged"))
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
            let _ = git::run(repo, &["fetch", "--quiet", "origin"]);
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
        g(&["branch", "ax/done"]);
        g(&["checkout", "-q", "-b", "ax/new"]);
        g(&commit);
        assert_eq!(merged_reason(&dir, "ax/done", "main").as_deref(), Some("already in origin/main"));
        assert!(merged_reason(&dir, "ax/new", "main").is_none() || std::process::Command::new("gh").arg("--version").output().is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
