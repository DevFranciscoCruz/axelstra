//! Integrates finished agent work into the project's default branch, so a worktree whose lead said `axelstra done` does
//! not stay behind as an empty row. `integrate` merges the `ax/*` branch (fast-forward when possible, else a clean
//! `--no-ff` merge that is aborted on conflict), optionally pushes, then removes the worktree and its branch. Anything
//! that cannot be done safely is an `Err` carrying an `integrate.*` code: nothing is changed, the worktree is flagged
//! ("needs attention") and the project's main agent is told. The janitor calls this for worktrees whose session
//! reported `done` (`mark_done`); the user can also ask for it by hand (`worktree.integrate`).

use crate::projects::{self, Project};
use anyhow::Result;
use axelstra_core::msg::Coded;
use axelstra_core::{coded, git};
use std::collections::HashMap;
use std::path::Path;
use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How long a `done` mark waits for the worktree to stop being busy before it is forgotten.
const MARK_TTL: Duration = Duration::from_secs(30 * 60);

/// Worktree path (normalized) -> (the report was `ok`, when it arrived).
static DONE: Mutex<Option<HashMap<String, (bool, Instant)>>> = Mutex::new(None);
/// Worktree path (normalized) -> why it was not integrated.
static ATTENTION: Mutex<Option<HashMap<String, Coded>>> = Mutex::new(None);

fn key(path: &str) -> String {
    axelstra_core::paths::norm(path)
}

/// Remembers that a session reported in this worktree (`ok` is false for `--status failed|blocked`).
pub fn mark_done(path: &str, ok: bool) {
    DONE.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(Default::default).insert(key(path), (ok, Instant::now()));
}

/// The mark of this worktree, if any and not expired; `take` forgets it.
pub fn done_mark(path: &str, take: bool) -> Option<bool> {
    let mut g = DONE.lock().unwrap_or_else(|e| e.into_inner());
    let m = g.get_or_insert_with(Default::default);
    m.retain(|_, (_, at)| at.elapsed() < MARK_TTL);
    if take {
        m.remove(&key(path)).map(|x| x.0)
    } else {
        m.get(&key(path)).map(|x| x.0)
    }
}

pub fn set_attention(path: &str, why: Coded) {
    ATTENTION.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(Default::default).insert(key(path), why);
}

pub fn clear_attention(path: &str) {
    if let Some(m) = ATTENTION.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        m.remove(&key(path));
    }
}

/// Why this worktree was not integrated, until it is, or removed, or an agent works in it again.
pub fn attention(path: &str) -> Option<Coded> {
    ATTENTION.lock().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|m| m.get(&key(path)).cloned())
}

/// The first coded error in `e`, or a generic `integrate.failed` for anything else.
pub fn as_coded(e: &anyhow::Error, branch: &str) -> Coded {
    axelstra_core::msg::find(e).cloned().unwrap_or_else(|| {
        let detail = format!("{e:#}");
        Coded::new(
            "integrate.failed",
            format!("could not integrate {branch}: {detail}"),
            &[("branch", branch.to_string()), ("detail", detail)],
        )
    })
}

/// Whether the project integrates finished `ax/*` work by itself (on unless the user turned it off).
pub fn auto_on(p: &Project) -> bool {
    p.auto_integrate.unwrap_or(true)
}

// one value per integration, never stored in bulk: boxing would only churn every match on it
#[allow(clippy::large_enum_variant)]
#[derive(Debug, PartialEq)]
pub enum Outcome {
    /// The branch had no commits the default branch lacks: nothing to merge.
    Nothing,
    Integrated {
        into: String,
        /// `ff` or `merge`
        how: &'static str,
        /// `None` when the project does not push; `Some(false)` when the push failed (the work is integrated anyway)
        pushed: Option<bool>,
        /// the worktree and branch are gone
        removed: bool,
        push_error: Option<Coded>,
        /// labels of the sessions closed so the folder could go
        closed: Vec<String>,
        /// what `undo` needs: the default branch before and after, and the integrated branch and its tip
        prev_tip: String,
        new_tip: String,
        branch: String,
        branch_tip: String,
    },
}

/// What `undo` is given: the numbers an `Integrated` outcome reported.
pub struct UndoArgs {
    pub default_branch: String,
    pub prev_tip: String,
    pub new_tip: String,
    pub branch: String,
    pub branch_tip: String,
}

/// What `undo` did.
#[derive(Debug, PartialEq)]
pub struct Undone {
    /// the integrated branch exists again (it had been removed)
    pub branch_restored: bool,
    /// the path of the worktree put back, when one could be
    pub worktree: Option<String>,
}

/// Commits on `branch` that `default` lacks.
pub fn ahead_of(repo: &Path, branch: &str, default: &str) -> u32 {
    git::run(repo, &["rev-list", "--count", &format!("{default}..{branch}")])
        .ok()
        .filter(|o| o.ok)
        .and_then(|o| o.stdout.trim().parse().ok())
        .unwrap_or(0)
}

/// Seconds since the branch was created (the oldest entry of its reflog); `None` when unknown.
pub fn branch_age_secs(repo: &Path, branch: &str) -> Option<u64> {
    let o = git::run(repo, &["log", "-g", "--format=%ct", branch]).ok().filter(|o| o.ok)?;
    let born: u64 = o.stdout.lines().last()?.trim().parse().ok()?;
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
    Some(now.saturating_sub(born))
}

/// Tracked files changed (or a merge half done): untracked files do not stop a merge, git refuses by itself when one
/// would be overwritten.
fn tracked_dirty(dir: &Path) -> bool {
    git::merge_in_progress(dir)
        || git::run(dir, &["status", "--porcelain", "--untracked-files=no"]).map(|o| !o.ok || !o.stdout.trim().is_empty()).unwrap_or(true)
}

/// Integrates the work of worktree `wt` of project `p`. The caller has checked that no session works there.
pub fn integrate(p: &Project, wt: &git::Worktree) -> Result<Outcome> {
    integrate_with(p, wt, &Vec::new)
}

/// `integrate`; `close_sessions` runs just before the folder is removed (never when the merge fails) and closes whatever
/// session still has it open, returning their labels.
pub fn integrate_with(p: &Project, wt: &git::Worktree, close_sessions: &dyn Fn() -> Vec<String>) -> Result<Outcome> {
    let repo = Path::new(&p.path);
    let dir = Path::new(&wt.path);
    let branch = wt.branch.clone().filter(|b| crate::janitor::integrable_branch(b)).ok_or_else(|| {
        coded!(
            "integrate.not_agent_branch",
            "only ax/* branches are integrated, this one is {branch}",
            branch = wt.branch.clone().unwrap_or_default()
        )
    })?;
    if wt.is_main {
        return Err(coded!("integrate.not_agent_branch", "only ax/* branches are integrated, this one is {branch}", branch = branch));
    }
    if git::is_pr_code(dir) {
        return Err(coded!(
            "integrate.untrusted",
            "{branch} holds code that is not Axelstra's own and is never integrated by itself",
            branch = branch
        ));
    }
    if tracked_dirty(dir) || git::has_changes(dir).unwrap_or(true) {
        return Err(coded!("integrate.dirty", "{branch} has uncommitted changes", branch = branch));
    }
    let default = git::default_branch(repo);
    if ahead_of(repo, &branch, &default) == 0 {
        return Ok(Outcome::Nothing);
    }
    // checked out in the main checkout (or another worktree): merge there, never touching a dirty one. Checked out
    // nowhere (the main checkout sits on another branch): move the ref without touching any checkout.
    // a checkout done outside Axelstra (`git checkout`) is not an event we see: never trust the cached list here
    git::forget_worktree_lists();
    let prev_tip = rev(repo, &format!("refs/heads/{default}")).map_err(|e| failed(&branch, &e))?;
    let branch_tip = rev(repo, &format!("refs/heads/{branch}")).map_err(|e| failed(&branch, &e))?;
    let host = git::worktree_list(repo)?.into_iter().find(|w| w.branch.as_deref() == Some(default.as_str()));
    let how = match &host {
        Some(host) => {
            let host_dir = Path::new(&host.path);
            if tracked_dirty(host_dir) {
                return Err(coded!("integrate.default_dirty", "{into} has uncommitted changes in its checkout", into = default));
            }
            if git::run(host_dir, &["merge", "--ff-only", "--quiet", &branch]).map(|o| o.ok).unwrap_or(false) {
                "ff"
            } else {
                let msg = format!("Merge {branch}");
                match git::merge(host_dir, &branch, &msg) {
                    Ok(git::Merge::Clean) => "merge",
                    Ok(git::Merge::Conflict) => return Err(conflict(&branch, &default)),
                    Err(e) => return Err(failed(&branch, &e)),
                }
            }
        }
        None => advance_unchecked(repo, &branch, &default)?,
    };
    let host_dir = host.as_ref().map(|h| Path::new(&h.path)).unwrap_or(repo);
    let new_tip = rev(repo, &format!("refs/heads/{default}")).map_err(|e| failed(&branch, &e))?;
    let (pushed, push_error) = if p.push_after_integrate {
        match push(host_dir, &default) {
            Ok(()) => (Some(true), None),
            Err(c) => (Some(false), Some(c)),
        }
    } else {
        (None, None)
    };
    // its commits are in the default branch now; a failure here only leaves the worktree for the next sweep
    let closed = close_sessions();
    let removed = projects::worktree_remove_strict(p, &wt.path).is_ok();
    Ok(Outcome::Integrated { into: default, how, pushed, removed, push_error, closed, prev_tip, new_tip, branch, branch_tip })
}

fn hex_oid(s: &str) -> bool {
    matches!(s.len(), 40 | 64) && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Takes back an integration made by `integrate_with`, as long as nothing was built on it: the default branch is still at
/// `new_tip`, no remote default branch has it (it was not pushed) and its checkout, if any, has no changes. The ref goes
/// back to `prev_tip` with a compare-and-swap; a checkout of the default branch is reset to it (it was clean), and when
/// the default branch is checked out nowhere no checkout is touched. If the integrated branch was removed it is recreated
/// at `branch_tip` (an `ax/*` one also gets its worktree back when the folder is free). Nothing is lost: `branch_tip` is
/// reachable from `new_tip` until the ref moves, and the branch is made again before it does.
pub fn undo(p: &Project, a: &UndoArgs) -> Result<Undone> {
    let repo = Path::new(&p.path);
    let (default, branch) = (a.default_branch.as_str(), a.branch.as_str());
    let bad = || coded!("integrate.undo_invalid", "the integration to undo is not valid");
    if ![&a.prev_tip, &a.new_tip, &a.branch_tip].iter().all(|s| hex_oid(s)) || !crate::janitor::integrable_branch(branch) {
        return Err(bad());
    }
    let valid_name = |n: &str| !n.starts_with('-') && git::run(repo, &["check-ref-format", "--branch", n]).map(|o| o.ok).unwrap_or(false);
    if !valid_name(default) || !valid_name(branch) || default == branch {
        return Err(bad());
    }
    let default_ref = format!("refs/heads/{default}");
    let branch_ref = format!("refs/heads/{branch}");
    if rev(repo, &default_ref).ok().as_deref() != Some(a.new_tip.as_str()) {
        return Err(coded!("integrate.undo_moved", "{into} changed after the integration, so it cannot be undone safely", into = default));
    }
    if !git::run(repo, &["merge-base", "--is-ancestor", &a.prev_tip, &a.new_tip]).map(|o| o.ok).unwrap_or(false) {
        return Err(bad());
    }
    let remote_has = git::run(repo, &["for-each-ref", "--format=%(refname)", "--contains", &a.new_tip, "refs/remotes"])
        .map(|o| {
            // refs/remotes/<remote>/<default>
            o.ok && o
                .stdout
                .lines()
                .any(|l| l.trim().strip_prefix("refs/remotes/").and_then(|r| r.split_once('/')).is_some_and(|(_, b)| b == default))
        })
        .unwrap_or(true);
    if remote_has {
        return Err(coded!("integrate.undo_pushed", "{into} was already pushed, so the integration cannot be undone", into = default));
    }
    git::forget_worktree_lists();
    let host = git::worktree_list(repo)?.into_iter().find(|w| w.branch.as_deref() == Some(default));
    if let Some(h) = &host {
        if tracked_dirty(Path::new(&h.path)) {
            return Err(coded!("integrate.default_dirty", "{into} has uncommitted changes in its checkout", into = default));
        }
    }
    // the branch comes back first: if that fails nothing has moved
    let existing = rev(repo, &branch_ref).ok();
    let mut branch_restored = false;
    if existing.is_none() {
        if !git::run(repo, &["cat-file", "-e", &format!("{}^{{commit}}", a.branch_tip)]).map(|o| o.ok).unwrap_or(false) {
            return Err(bad());
        }
        git::must(repo, &["update-ref", &branch_ref, &a.branch_tip, ""])?;
        branch_restored = true;
    }
    if let Err(e) = cas_ref(repo, default, branch, &a.prev_tip, &a.new_tip) {
        if branch_restored {
            let _ = git::run(repo, &["update-ref", "-d", &branch_ref, &a.branch_tip]);
        }
        return Err(e);
    }
    if let Some(h) = &host {
        // the ref moved under a clean checkout: bring its index and files along
        if let Err(e) = git::must(Path::new(&h.path), &["reset", "--hard", "--quiet", &a.prev_tip]) {
            let _ = git::run(repo, &["update-ref", &default_ref, &a.new_tip, &a.prev_tip]);
            if branch_restored {
                let _ = git::run(repo, &["update-ref", "-d", &branch_ref, &a.branch_tip]);
            }
            return Err(failed(branch, &e));
        }
    }
    let worktree = if branch_restored && branch.starts_with("ax/") {
        match projects::worktree_attach(p, branch) {
            Ok(w) => Some(w.path),
            Err(e) => {
                eprintln!("undo of {branch}: no worktree put back: {e:#}");
                None
            }
        }
    } else {
        None
    };
    git::forget_worktree_lists();
    Ok(Undone { branch_restored, worktree })
}

fn conflict(branch: &str, into: &str) -> anyhow::Error {
    coded!("integrate.conflict", "merging {branch} into {into} has conflicts; nothing was changed", branch = branch, into = into)
}

fn failed(branch: &str, e: &anyhow::Error) -> anyhow::Error {
    coded!("integrate.failed", "could not integrate {branch}: {detail}", branch = branch, detail = format!("{e:#}"))
}

fn rev(repo: &Path, spec: &str) -> Result<String> {
    Ok(git::must(repo, &["rev-parse", "--verify", "--quiet", spec])?.trim().to_string())
}

/// `git update-ref refs/heads/<default> <new> <old>`: fails with `integrate.default_moved` when the ref is no longer `old`.
pub fn cas_ref(repo: &Path, default: &str, branch: &str, new: &str, old: &str) -> Result<()> {
    match git::run(repo, &["update-ref", &format!("refs/heads/{default}"), new, old]) {
        Ok(o) if o.ok => Ok(()),
        _ => Err(coded!(
            "integrate.default_moved",
            "{into} changed while {branch} was being integrated; nothing was changed, try again",
            into = default,
            branch = branch
        )),
    }
}

/// Moves `refs/heads/<default>` to include `branch` while `default` is checked out nowhere. Fast-forward: the ref is
/// updated with a compare-and-swap, no checkout is touched. Otherwise the merge happens in a throwaway detached worktree
/// that is always removed. Returns `ff` or `merge`; the ref moved only when it is `Ok`.
fn advance_unchecked(repo: &Path, branch: &str, default: &str) -> Result<&'static str> {
    let default_ref = format!("refs/heads/{default}");
    let (old, tip) = match (rev(repo, &default_ref), rev(repo, &format!("refs/heads/{branch}"))) {
        (Ok(o), Ok(t)) => (o, t),
        (Err(e), _) | (_, Err(e)) => return Err(failed(branch, &e)),
    };
    let swap = |new: &str| cas_ref(repo, default, branch, new, &old);
    if git::run(repo, &["merge-base", "--is-ancestor", &old, &tip]).map(|o| o.ok).unwrap_or(false) {
        swap(&tip)?;
        return Ok("ff");
    }
    let tmp = std::env::temp_dir().join(format!(
        "axl-integ-{}-{}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0)
    ));
    let result = (|| -> Result<&'static str> {
        git::must(repo, &["-c", "core.longpaths=true", "worktree", "add", "--detach", "-q", &tmp.to_string_lossy(), &old])?;
        match git::merge(&tmp, branch, &format!("Merge {branch}")) {
            Ok(git::Merge::Clean) => {
                swap(git::head(&tmp)?.trim())?;
                Ok("merge")
            }
            Ok(git::Merge::Conflict) => Err(conflict(branch, default)),
            Err(e) => Err(failed(branch, &e)),
        }
    })();
    let _ = git::run(repo, &["worktree", "remove", "--force", &tmp.to_string_lossy()]);
    let _ = git::remove_tree(&tmp);
    let _ = git::run(repo, &["worktree", "prune"]);
    git::forget_worktree_lists();
    result
}

fn push(dir: &Path, default: &str) -> std::result::Result<(), Coded> {
    let mut cmd = Command::new("git");
    cmd.current_dir(dir).env("GIT_TERMINAL_PROMPT", "0").args(["push", "origin", default]);
    match crate::janitor::output_within(&mut cmd, Duration::from_secs(120)) {
        Some(o) if o.success() => Ok(()),
        other => {
            let detail = other.map(|o| o.stderr_lossy().trim().to_string()).unwrap_or_else(|| "timeout".into());
            Err(Coded::new(
                "integrate.push_failed",
                format!("{default} was integrated but the push failed: {detail}"),
                &[("into", default.to_string()), ("detail", detail)],
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fx {
        root: std::path::PathBuf,
        store: std::path::PathBuf,
        p: Project,
    }

    impl Drop for Fx {
        fn drop(&mut self) {
            git::purge_trash(&self.root);
            let _ = git::remove_tree(&self.root);
        }
    }

    fn g(dir: &Path, args: &[&str]) {
        let mut a = vec!["-c", "user.name=t", "-c", "user.email=t@t", "-c", "core.autocrlf=false"];
        a.extend_from_slice(args);
        let o = git::run(dir, &a).unwrap();
        assert!(o.ok, "git {args:?}: {}", o.stderr);
    }

    fn fx(tag: &str) -> Fx {
        let root = std::env::temp_dir().join(format!("axl-int-{tag}-{}", std::process::id()));
        let _ = git::remove_tree(&root);
        let repo = root.join("proj");
        std::fs::create_dir_all(&repo).unwrap();
        g(&repo, &["init", "-q", "-b", "main"]);
        std::fs::write(repo.join("base.txt"), "base\n").unwrap();
        g(&repo, &["add", "base.txt"]);
        g(&repo, &["commit", "-q", "-m", "base"]);
        let store = root.join("projects.json");
        let p = projects::add(&store, &repo.to_string_lossy()).unwrap();
        Fx { root, store, p }
    }

    fn work(dir: &Path, file: &str, text: &str) {
        std::fs::write(dir.join(file), text).unwrap();
        g(dir, &["add", file]);
        g(dir, &["commit", "-q", "-m", file]);
    }

    fn wt_of(p: &Project, w: &git::Worktree) -> git::Worktree {
        git::worktree_list(Path::new(&p.path))
            .unwrap()
            .into_iter()
            .find(|x| x.path == w.path || axelstra_core::paths::same(&x.path, &w.path))
            .unwrap()
    }

    fn code(e: anyhow::Error) -> String {
        axelstra_core::msg::find(&e).map(|c| c.code.clone()).unwrap_or_else(|| format!("{e:#}"))
    }

    #[test]
    fn fast_forwards_and_removes_the_worktree_and_branch() {
        let f = fx("ff");
        let w = projects::worktree_create(&f.p, "ff").unwrap();
        work(Path::new(&w.path), "a.txt", "a\n");
        let out = integrate(&f.p, &wt_of(&f.p, &w)).unwrap();
        match out {
            Outcome::Integrated { into, how, pushed, removed, .. } => {
                assert_eq!((into.as_str(), how, pushed, removed), ("main", "ff", None, true))
            }
            o => panic!("{o:?}"),
        }
        assert!(Path::new(&f.p.path).join("a.txt").exists());
        assert!(!Path::new(&w.path).exists());
        assert!(!git::run(Path::new(&f.p.path), &["rev-parse", "--verify", "ax/ff"]).unwrap().ok, "branch deleted");
        let _ = &f.store;
    }

    #[test]
    fn a_diverged_branch_gets_a_clean_merge_commit() {
        let f = fx("merge");
        let repo = Path::new(&f.p.path);
        let w = projects::worktree_create(&f.p, "m").unwrap();
        work(Path::new(&w.path), "a.txt", "a\n");
        work(repo, "b.txt", "b\n");
        let out = integrate(&f.p, &wt_of(&f.p, &w)).unwrap();
        assert!(matches!(out, Outcome::Integrated { how: "merge", removed: true, .. }), "{out:?}");
        assert!(repo.join("a.txt").exists() && repo.join("b.txt").exists());
        let parents = git::run(repo, &["rev-list", "--parents", "-n", "1", "HEAD"]).unwrap().stdout;
        assert_eq!(parents.split_whitespace().count(), 3, "a merge commit has two parents: {parents}");
        assert!(!Path::new(&w.path).exists());
    }

    #[test]
    fn a_conflict_changes_nothing() {
        let f = fx("conflict");
        let repo = Path::new(&f.p.path);
        let w = projects::worktree_create(&f.p, "c").unwrap();
        work(Path::new(&w.path), "base.txt", "theirs\n");
        work(repo, "base.txt", "ours\n");
        let before = git::head(repo).unwrap();
        let e = integrate(&f.p, &wt_of(&f.p, &w)).unwrap_err();
        assert_eq!(code(e), "integrate.conflict");
        assert_eq!(git::head(repo).unwrap(), before);
        assert!(!git::merge_in_progress(repo));
        assert_eq!(std::fs::read_to_string(repo.join("base.txt")).unwrap().trim(), "ours");
        assert!(Path::new(&w.path).exists());
    }

    #[test]
    fn a_dirty_worktree_or_dirty_main_checkout_is_refused() {
        let f = fx("dirty");
        let repo = Path::new(&f.p.path);
        let w = projects::worktree_create(&f.p, "d").unwrap();
        work(Path::new(&w.path), "a.txt", "a\n");
        std::fs::write(Path::new(&w.path).join("wip.txt"), "w").unwrap();
        assert_eq!(code(integrate(&f.p, &wt_of(&f.p, &w)).unwrap_err()), "integrate.dirty");
        std::fs::remove_file(Path::new(&w.path).join("wip.txt")).unwrap();
        // a tracked file changed in the main checkout
        std::fs::write(repo.join("base.txt"), "edited\n").unwrap();
        assert_eq!(code(integrate(&f.p, &wt_of(&f.p, &w)).unwrap_err()), "integrate.default_dirty");
        assert!(Path::new(&w.path).exists());
        // an untracked file in the main checkout does not block
        g(repo, &["checkout", "-q", "--", "base.txt"]);
        std::fs::write(repo.join("scratch.txt"), "s").unwrap();
        assert!(matches!(integrate(&f.p, &wt_of(&f.p, &w)).unwrap(), Outcome::Integrated { how: "ff", .. }));
    }

    #[test]
    fn an_empty_branch_has_nothing_to_integrate_and_pr_code_is_never_integrated() {
        let f = fx("empty");
        let w = projects::worktree_create(&f.p, "e").unwrap();
        assert_eq!(integrate(&f.p, &wt_of(&f.p, &w)).unwrap(), Outcome::Nothing);
        assert!(Path::new(&w.path).exists());
        work(Path::new(&w.path), "a.txt", "a\n");
        git::mark_untrusted(Path::new(&w.path)).unwrap();
        assert_eq!(code(integrate(&f.p, &wt_of(&f.p, &w)).unwrap_err()), "integrate.untrusted");
    }

    #[test]
    fn push_after_integrate_pushes_only_when_asked() {
        let mut f = fx("push");
        let bare = f.root.join("origin.git");
        std::fs::create_dir_all(&bare).unwrap();
        g(&bare, &["init", "-q", "--bare", "-b", "main"]);
        let repo = std::path::PathBuf::from(&f.p.path);
        g(&repo, &["remote", "add", "origin", &bare.to_string_lossy()]);
        g(&repo, &["push", "-q", "origin", "main"]);
        let w = projects::worktree_create(&f.p, "p1").unwrap();
        work(Path::new(&w.path), "a.txt", "a\n");
        assert!(matches!(integrate(&f.p, &wt_of(&f.p, &w)).unwrap(), Outcome::Integrated { pushed: None, .. }));
        let remote = |r: &str| git::run(&bare, &["rev-parse", r]).unwrap().stdout.trim().to_string();
        assert_ne!(remote("main"), git::head(&repo).unwrap(), "no push without the setting");
        f.p.push_after_integrate = true;
        let w2 = projects::worktree_create(&f.p, "p2").unwrap();
        work(Path::new(&w2.path), "b.txt", "b\n");
        assert!(matches!(integrate(&f.p, &wt_of(&f.p, &w2)).unwrap(), Outcome::Integrated { pushed: Some(true), .. }));
        assert_eq!(remote("main"), git::head(&repo).unwrap());
    }

    #[test]
    fn marks_expire_and_are_taken_once() {
        mark_done("/x/one", true);
        assert_eq!(done_mark("/x/one", false), Some(true));
        assert_eq!(done_mark("/x/one", true), Some(true));
        assert_eq!(done_mark("/x/one", true), None);
        set_attention("/x/two", Coded::new("integrate.dirty", "d".into(), &[]));
        assert_eq!(attention("/x/two").unwrap().code, "integrate.dirty");
        clear_attention("/x/two");
        assert!(attention("/x/two").is_none());
    }
}
