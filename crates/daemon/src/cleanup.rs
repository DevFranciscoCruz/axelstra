//! The parts of the automatic clean-up that are not "remove the worktree of finished work" (that is `janitor`):
//! work that landed through a squash (no ancestor, found with `git cherry`), `ax/*` branches nobody has checked out,
//! folders in `.axelstra-wt/` that git no longer knows (what a failed "Filename too long" removal leaves), stale
//! `git worktree prune` entries, and the rotation of `backups/` and the logs. Everything here only removes what holds
//! no work: a branch with commits that are not in the default branch, or a folder git still lists, is never touched.

use crate::projects::Project;
use axelstra_core::git;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// Backups kept in `backups/` (the newest ones).
pub const KEEP_BACKUPS: usize = 20;
/// A log larger than this is moved to `<name>.1` (replacing the previous one) the next time the sweep runs.
pub const MAX_LOG_BYTES: u64 = 5 << 20;
/// Files of `sid/` and `scrollback/` nobody refers to are kept this long before they go (a session being saved).
pub const LEFTOVER_MIN_AGE_SECS: u64 = 60 * 60;

fn item(p: Option<&Project>, path: &str, branch: &str, kind: &str, why: &str, removed: bool, dry: bool) -> Value {
    json!({"project": p.map(|p| p.id.as_str()).unwrap_or(""), "path": path, "branch": branch, "kind": kind, "why": why,
           "removed": removed, "dry_run": dry})
}

/// A refusal to delete is shown, not swallowed: `warning` carries the reason.
fn warn(p: Option<&Project>, path: &str, kind: &str, why: String, dry: bool) -> Value {
    let mut v = item(p, path, "", kind, &why, false, dry);
    v["warning"] = json!(why);
    v
}

/// True when every commit of `branch` has an equivalent in `upstream` (`git cherry` marks them all `-`), or merging
/// the branch into it would change nothing: the signature of a squash or rebase merge, which leaves no ancestor.
pub fn squash_merged(repo: &Path, branch: &str, upstream: &str) -> bool {
    if let Ok(o) = git::run(repo, &["cherry", upstream, branch]) {
        if o.ok {
            let lines: Vec<&str> = o.stdout.lines().filter(|l| !l.trim().is_empty()).collect();
            if !lines.is_empty() && lines.iter().all(|l| l.starts_with('-')) {
                return true;
            }
        }
    }
    // several commits squashed into one are not "equivalent" one by one, but their combined change is already there
    let tree = |rev: &str| git::run(repo, &["rev-parse", rev]).ok().filter(|o| o.ok).map(|o| o.stdout.trim().to_string());
    match (git::run(repo, &["merge-tree", "--write-tree", upstream, branch]), tree(&format!("{upstream}^{{tree}}"))) {
        (Ok(m), Some(t)) if m.ok => m.stdout.lines().next().map(str::trim) == Some(t.as_str()),
        _ => false,
    }
}

/// Local `ax/*` branches that no worktree has checked out and whose work is in the default branch (or that never got a
/// commit of their own) are deleted. Returns what was done.
pub fn stale_branches(p: &Project, dry: bool, out: &mut Vec<Value>) {
    let repo = Path::new(&p.path);
    let default = git::default_branch(repo);
    let Ok(wts) = git::worktree_list(repo) else { return };
    let used: Vec<String> = wts.into_iter().filter_map(|w| w.branch).collect();
    let Ok(o) = git::run(repo, &["for-each-ref", "--format=%(refname:short)", "refs/heads/ax/"]) else { return };
    for b in o.stdout.lines().map(str::trim).filter(|b| !b.is_empty() && !used.iter().any(|u| u == b)) {
        let why = match crate::janitor::merged_reason_in(repo, b, &default, &[]) {
            Some(w) => w,
            None if !crate::janitor::has_own_commits(repo, b) && crate::integrate::ahead_of(repo, b, &default) == 0 => "empty".to_string(),
            None => continue,
        };
        let removed = !dry && git::run(repo, &["branch", "-D", b]).map(|o| o.ok).unwrap_or(false);
        if !dry && !removed {
            out.push(warn(Some(p), "", "branch", format!("could not delete the branch {b}"), dry));
            continue;
        }
        out.push(item(Some(p), "", b, "branch", &why, removed, dry));
    }
}

/// `git worktree prune` for entries whose folder is gone; true when something was pruned (a dry run only looks).
pub fn prune_entries(p: &Project, dry: bool, out: &mut Vec<Value>) {
    let repo = Path::new(&p.path);
    let args: &[&str] = if dry { &["worktree", "prune", "--dry-run", "-v"] } else { &["worktree", "prune", "-v"] };
    let Ok(o) = git::run(repo, args) else { return };
    // `-v` prints one line per entry it removes (or would remove) on stderr
    for l in o.stderr.lines().chain(o.stdout.lines()).filter(|l| l.contains("Removing")) {
        out.push(item(Some(p), l.trim(), "", "prune", "git no longer has the folder", !dry, dry));
    }
}

/// Folders of `.axelstra-wt/` next to the projects' repos that no project lists as a worktree, and whose `.git` does
/// not lead to a live repository. `busy(path)` says whether a session still works inside one. Folders parked by an
/// earlier removal (`.axelstra-trash-*`) are emptied too.
pub fn orphan_dirs(projects: &[Project], busy: &dyn Fn(&str) -> bool, dry: bool, out: &mut Vec<Value>) {
    let mut roots: Vec<PathBuf> = vec![];
    let mut listed: Vec<String> = vec![];
    for p in projects {
        let repo = Path::new(&p.path);
        let Some(root) = repo.parent().map(|r| r.join(".axelstra-wt")) else { continue };
        // a project git cannot list now (repo moved, git failing) makes the whole folder unsafe to judge
        let Ok(wts) = git::worktree_list(repo) else { return };
        listed.extend(wts.iter().map(|w| crate::projects::norm_path(&w.path)));
        if !roots.contains(&root) {
            roots.push(root);
        }
    }
    for root in roots {
        let Ok(rd) = std::fs::read_dir(git::verbatim(&root)) else { continue };
        for e in rd.flatten() {
            let path = root.join(e.file_name());
            let name = e.file_name().to_string_lossy().to_string();
            let ps = path.to_string_lossy().to_string();
            if !e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }
            if name.starts_with(".axelstra-trash-") {
                if !dry {
                    let _ = git::remove_tree(&path);
                }
                continue;
            }
            if name.starts_with('.') || listed.contains(&crate::projects::norm_path(&ps)) || busy(&ps) {
                continue;
            }
            // a `.git` file pointing at a folder that still exists: some repo (maybe another project) owns it
            let gitfile = git::verbatim(&path).join(".git");
            if let Ok(txt) = std::fs::read_to_string(&gitfile) {
                let target = txt.trim().strip_prefix("gitdir:").map(|t| PathBuf::from(t.trim()));
                if target.is_some_and(|t| t.exists()) {
                    continue;
                }
            }
            if dry {
                out.push(item(None, &ps, "", "orphan", "not a worktree of any project", false, true));
                continue;
            }
            match git::remove_tree(&path) {
                Ok(()) if !git::verbatim(&path).exists() => {
                    out.push(item(None, &ps, "", "orphan", "not a worktree of any project", true, false))
                }
                Ok(()) => out.push(warn(None, &ps, "orphan", "the folder is still there after removing it".into(), false)),
                Err(e) => out.push(warn(None, &ps, "orphan", format!("could not remove the folder: {e}"), false)),
            }
        }
    }
}

/// Keeps the newest `keep` files of `dir`.
fn keep_newest(dir: &Path, keep: usize, dry: bool, out: &mut Vec<Value>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = rd
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .collect();
    files.sort_by_key(|a| std::cmp::Reverse(a.0));
    for (_, f) in files.into_iter().skip(keep) {
        let ps = f.to_string_lossy().to_string();
        match if dry { Ok(()) } else { std::fs::remove_file(&f) } {
            Ok(()) => out.push(item(None, &ps, "", "backup", "older than the newest backups", !dry, dry)),
            Err(e) => out.push(warn(None, &ps, "backup", format!("could not delete it: {e}"), dry)),
        }
    }
}

/// Rotation of `backups/` and of the logs in the data dir.
pub fn rotate(data_dir: &Path, dry: bool, out: &mut Vec<Value>) {
    keep_newest(&data_dir.join("backups"), KEEP_BACKUPS, dry, out);
    for log in ["crash.log", "lag.log", "audit.log"] {
        let f = data_dir.join(log);
        if std::fs::metadata(&f).map(|m| m.len() > MAX_LOG_BYTES).unwrap_or(false) {
            let ps = f.to_string_lossy().to_string();
            let old = data_dir.join(format!("{log}.1"));
            let moved = if dry {
                Ok(())
            } else {
                let _ = std::fs::remove_file(&old);
                std::fs::rename(&f, &old)
            };
            match moved {
                Ok(()) => out.push(item(None, &ps, "", "log", "larger than the log limit", !dry, dry)),
                Err(e) => out.push(warn(None, &ps, "log", format!("could not rotate it: {e}"), dry)),
            }
        }
    }
}

/// Files of `sid/` and `scrollback/` whose session key is not in `keys` (no saved or live session refers to them) and
/// that are older than `LEFTOVER_MIN_AGE_SECS`.
pub fn leftovers(data_dir: &Path, keys: &[String], dry: bool, out: &mut Vec<Value>) {
    for (sub, ext) in [("sid", "json"), ("scrollback", "txt")] {
        let Ok(rd) = std::fs::read_dir(data_dir.join(sub)) else { continue };
        for e in rd.flatten() {
            let path = e.path();
            let Some(key) = path.file_name().and_then(|n| n.to_str()).and_then(|n| n.strip_suffix(&format!(".{ext}"))).map(String::from)
            else {
                continue;
            };
            let old = e
                .metadata()
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.elapsed().ok())
                .is_some_and(|a| a.as_secs() >= LEFTOVER_MIN_AGE_SECS);
            if keys.contains(&key) || !old {
                continue;
            }
            let ps = path.to_string_lossy().to_string();
            match if dry { Ok(()) } else { std::fs::remove_file(&path) } {
                Ok(()) => out.push(item(None, &ps, "", "session", "no session refers to it", !dry, dry)),
                Err(e) => out.push(warn(None, &ps, "session", format!("could not delete it: {e}"), dry)),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fx(PathBuf);
    impl Drop for Fx {
        fn drop(&mut self) {
            let _ = git::remove_tree(&self.0);
        }
    }

    fn g(dir: &Path, args: &[&str]) {
        let mut a = vec!["-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false"];
        a.extend_from_slice(args);
        assert!(git::run(dir, &a).unwrap().ok, "git {args:?}");
    }

    /// repo `<root>/repo` on `main` with one commit, plus the project that points at it
    fn fx(tag: &str) -> (Fx, Project) {
        let root = std::env::temp_dir().join(format!("axl-clean-{tag}-{}", std::process::id()));
        let _ = git::remove_tree(&root);
        let repo = root.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        g(&repo, &["init", "-q", "-b", "main"]);
        std::fs::write(repo.join("a.txt"), "a\n").unwrap();
        g(&repo, &["add", "."]);
        g(&repo, &["commit", "-qm", "init"]);
        let path = git::canon(&repo).unwrap().to_string_lossy().to_string();
        (Fx(root), Project { id: "p".into(), name: "repo".into(), path, ..Default::default() })
    }

    fn repo(p: &Project) -> &Path {
        Path::new(&p.path)
    }

    #[test]
    fn a_squash_merged_branch_is_found_without_an_ancestor_and_unmerged_work_is_not() {
        let (_f, p) = fx("squash");
        let r = repo(&p);
        g(r, &["checkout", "-qb", "ax/one"]);
        for n in ["1", "2"] {
            std::fs::write(r.join(format!("f{n}.txt")), n).unwrap();
            g(r, &["add", "."]);
            g(r, &["commit", "-qm", n]);
        }
        g(r, &["checkout", "-q", "main"]);
        g(r, &["merge", "--squash", "ax/one"]);
        g(r, &["commit", "-qm", "squashed"]);
        assert!(git::run(r, &["merge-base", "--is-ancestor", "ax/one", "main"]).map(|o| !o.ok).unwrap_or(false), "no ancestor");
        assert!(squash_merged(r, "ax/one", "main"));
        g(r, &["checkout", "-qb", "ax/two"]);
        std::fs::write(r.join("only-here.txt"), "x").unwrap();
        g(r, &["add", "."]);
        g(r, &["commit", "-qm", "unmerged"]);
        g(r, &["checkout", "-q", "main"]);
        assert!(!squash_merged(r, "ax/two", "main"));
    }

    #[test]
    fn stale_branches_go_only_when_merged_or_empty() {
        let (_f, p) = fx("branches");
        let r = repo(&p);
        g(r, &["checkout", "-qb", "ax/merged"]);
        std::fs::write(r.join("m.txt"), "m").unwrap();
        g(r, &["add", "."]);
        g(r, &["commit", "-qm", "m"]);
        g(r, &["checkout", "-q", "main"]);
        g(r, &["merge", "--squash", "ax/merged"]);
        g(r, &["commit", "-qm", "squashed"]);
        g(r, &["checkout", "-qb", "ax/open"]);
        std::fs::write(r.join("o.txt"), "o").unwrap();
        g(r, &["add", "."]);
        g(r, &["commit", "-qm", "o"]);
        g(r, &["checkout", "-q", "main"]);
        g(r, &["branch", "ax/empty"]);
        let mut out = vec![];
        stale_branches(&p, true, &mut out);
        assert!(out.iter().all(|o| o["removed"] == false), "a dry run removes nothing");
        assert_eq!(out.len(), 2);
        let mut out = vec![];
        stale_branches(&p, false, &mut out);
        let left = git::run(r, &["branch", "--list", "ax/*"]).unwrap().stdout;
        assert!(left.contains("ax/open") && !left.contains("ax/merged") && !left.contains("ax/empty"), "{left}");
    }

    #[test]
    fn an_orphan_folder_with_a_path_over_260_characters_is_removed_and_a_listed_worktree_stays() {
        let (f, p) = fx("orphan");
        let r = repo(&p);
        let root = f.0.join(".axelstra-wt");
        // a real worktree, listed by git
        let live = root.join("repo-live");
        std::fs::create_dir_all(&root).unwrap();
        g(r, &["worktree", "add", "-q", "-b", "ax/live", &live.to_string_lossy()]);
        // what a failed removal leaves: git forgot the folder, files stay under a very deep path
        let orphan = root.join("repo-orphan");
        let mut deep = git::verbatim(&orphan);
        for i in 0..12 {
            deep = deep.join(format!("level-{i}-{}", "x".repeat(24)));
        }
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::write(deep.join("big.bin"), "x").unwrap();
        assert!(deep.to_string_lossy().len() > 260);
        // and a folder some session still works in
        let busy_dir = root.join("repo-busy");
        std::fs::create_dir_all(&busy_dir).unwrap();
        let busy_s = busy_dir.to_string_lossy().to_string();
        let busy = |path: &str| crate::projects::norm_path(path) == crate::projects::norm_path(&busy_s);
        let mut out = vec![];
        orphan_dirs(std::slice::from_ref(&p), &busy, true, &mut out);
        assert!(out.iter().any(|o| o["kind"] == "orphan" && o["path"].as_str().unwrap().ends_with("repo-orphan")), "{out:?}");
        assert!(git::verbatim(&orphan).exists(), "a dry run keeps it");
        let mut out = vec![];
        orphan_dirs(std::slice::from_ref(&p), &busy, false, &mut out);
        assert!(!git::verbatim(&orphan).exists(), "{out:?}");
        assert!(live.exists() && busy_dir.exists(), "listed and busy folders stay");
        assert!(out.iter().all(|o| o.get("warning").is_none()), "{out:?}");
    }

    #[test]
    fn prune_clears_entries_of_folders_that_are_gone() {
        let (f, p) = fx("prune");
        let r = repo(&p);
        let wt = f.0.join("wt-gone");
        g(r, &["worktree", "add", "-q", "-b", "ax/gone", &wt.to_string_lossy()]);
        std::fs::remove_dir_all(&wt).unwrap();
        let mut out = vec![];
        prune_entries(&p, false, &mut out);
        assert_eq!(out.len(), 1, "{out:?}");
        assert!(!git::run(r, &["worktree", "list"]).unwrap().stdout.contains("wt-gone"));
    }

    #[test]
    fn rotation_keeps_the_newest_backups_and_moves_a_big_log_aside_and_leftovers_need_age_and_no_owner() {
        let dir = std::env::temp_dir().join(format!("axl-clean-rot-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("backups")).unwrap();
        for i in 0..(KEEP_BACKUPS + 3) {
            std::fs::write(dir.join("backups").join(format!("b{i}.json")), "{}").unwrap();
            std::thread::sleep(std::time::Duration::from_millis(15));
        }
        std::fs::write(dir.join("crash.log"), vec![b'x'; (MAX_LOG_BYTES + 1) as usize]).unwrap();
        std::fs::write(dir.join("lag.log"), "small").unwrap();
        let mut out = vec![];
        rotate(&dir, false, &mut out);
        assert_eq!(std::fs::read_dir(dir.join("backups")).unwrap().count(), KEEP_BACKUPS);
        assert!(!dir.join("backups").join("b0.json").exists() && dir.join("backups").join(format!("b{}.json", KEEP_BACKUPS + 2)).exists());
        assert!(!dir.join("crash.log").exists() && dir.join("crash.log.1").exists() && dir.join("lag.log").exists());
        // leftovers: a fresh file is kept whoever owns it, an old unowned one goes
        std::fs::create_dir_all(dir.join("sid")).unwrap();
        for k in ["fresh", "owned", "dead"] {
            std::fs::write(dir.join("sid").join(format!("{k}.json")), "{}").unwrap();
        }
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(LEFTOVER_MIN_AGE_SECS + 60);
        for k in ["owned", "dead"] {
            std::fs::File::options().write(true).open(dir.join("sid").join(format!("{k}.json"))).unwrap().set_modified(old).unwrap();
        }
        let mut out = vec![];
        leftovers(&dir, &["owned".to_string()], false, &mut out);
        assert!(dir.join("sid/fresh.json").exists() && dir.join("sid/owned.json").exists() && !dir.join("sid/dead.json").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
