//! What happens to an orchestrator run's worktrees (`orch.start`): the integration worktree `<name>-integ` on branch
//! `orch/<name>` and the task worktrees `<name>-<card>` on `orch-task/<name>/<card>`. They are Axelstra's own, so they are
//! marked `auto` before the run creates them (`mark_run`); the janitor then cleans them like an agent's worktree, and
//! leaves them alone while the run is active (`Registry::orch_active`). When the run ends (`finish`, called by the run's
//! thread, and again after the last `orch.retry`) nothing with work in it is deleted:
//! - a run that merged every card and left unintegrated commits on `orch/<name>` is integrated into the default branch
//!   (`integrate::integrate_with`: fast-forward or a clean merge); on a conflict, a dirty checkout and the like it stays as
//!   "por integrar" with the reason (`integrate::attention`) and the usual Integrar button; after a successful
//!   integration the worktree and branch are gone with it;
//! - a branch with no commits the default branch lacks (a failed run, a run that changed nothing) is removed at once;
//! - a run with failed, blocked or unreviewed cards that left commits keeps them, and says why (`orch.kept`);
//! - while a card waits for review (`holds`) nothing is touched: `orch.retry` needs both worktrees.

use crate::integrate;
use crate::janitor::{self, attention_event, push_event, removed_event};
use crate::projects::{self, Project};
use axelstra_core::git;
use axelstra_core::msg::Coded;
use serde_json::{json, Value};
use std::path::Path;

/// How the run ended, as far as cleaning up is concerned.
pub struct End {
    /// every card was merged and none failed, was blocked or waits for review
    pub clean: bool,
    /// a card waits for review: its worktree and the integration worktree are needed by `orch.retry`
    pub holds: bool,
}

/// Marks the worktrees the run is about to create as Axelstra's own (`auto`): the integration worktree and one per todo
/// card. A path that never gets created is harmless: the janitor only looks at what git lists.
pub fn mark_run(store: &Path, project: &str, repo: &Path, wt_root: &Path, name: &str) {
    let mut paths = vec![wt_root.join(format!("{name}-integ"))];
    if let Ok(info) = axelstra_core::orch::trust_info(repo) {
        paths.extend(info.cards.iter().map(|c| wt_root.join(format!("{name}-{}", c.id))));
    }
    for p in paths {
        if let Err(e) = projects::set_auto(store, project, &p.to_string_lossy()) {
            eprintln!("auto mark of {}: {e:#}", p.display());
        }
    }
}

/// Removes the worktree after closing the sessions in it; false when it stayed (a session left changes, or the folder is held).
fn remove(p: &Project, path: &str, close: &dyn Fn(&str) -> Vec<String>) -> (bool, Vec<String>) {
    let closed = close(path);
    if !closed.is_empty() && git::has_changes(Path::new(path)).unwrap_or(true) {
        return (false, closed);
    }
    (projects::worktree_remove_strict(p, path).is_ok(), closed)
}

fn kept(branch: &str) -> Coded {
    Coded::new(
        "orch.kept",
        format!("{branch} was kept: the run did not finish every card and the branch holds work. Review it, then integrate it."),
        &[("branch", branch.to_string())],
    )
}

/// Cleans up after the run `name` of project `p`; `close(path)` closes the sessions of a worktree and returns their labels.
/// Queues janitor events for what it did. Never removes a worktree that is dirty or whose branch holds unintegrated work.
pub fn finish(p: &Project, name: &str, end: &End, close: &dyn Fn(&str) -> Vec<String>) {
    if end.holds {
        return;
    }
    let repo = Path::new(&p.path);
    let Ok(list) = git::worktree_list(repo) else { return };
    let default = git::default_branch(repo);
    let integ_branch = format!("orch/{name}");
    let task_prefix = format!("orch-task/{name}/");
    // task worktrees first: the ones left behind with nothing in them (a spawn that failed half way)
    for w in list.iter().filter(|w| !w.is_main) {
        let Some(branch) = w.branch.as_deref().filter(|b| b.starts_with(&task_prefix)) else { continue };
        let empty = !git::has_changes(Path::new(&w.path)).unwrap_or(true)
            && !janitor::has_own_commits(repo, branch)
            && integrate::ahead_of(repo, branch, &integ_branch) == 0;
        if empty {
            remove(p, &w.path, close);
        }
    }
    let Some(w) = list.iter().find(|w| !w.is_main && w.branch.as_deref() == Some(integ_branch.as_str())) else { return };
    let dirty = git::has_changes(Path::new(&w.path)).unwrap_or(true);
    if integrate::ahead_of(repo, &integ_branch, &default) == 0 && !dirty {
        // nothing the default branch lacks: either the run changed nothing or its work is in already
        let own = janitor::has_own_commits(repo, &integ_branch);
        let (removed, closed) = remove(p, &w.path, close);
        if removed {
            push_event(if own {
                removed_event(p, &w.path, &integ_branch, &default, &format!("already in local {default}"), &closed)
            } else {
                json!({"kind": "empty", "project": p.id, "path": w.path, "branch": integ_branch, "closed": closed, "at": now_ms()})
            });
        }
        return;
    }
    if !end.clean {
        let why = kept(&integ_branch);
        integrate::set_attention(&w.path, why.clone());
        push_event(attention_event(p, &w.path, &integ_branch, &why));
        return;
    }
    if !integrate::auto_on(p) {
        return;
    }
    match integrate::integrate_with(p, w, &|| close(&w.path)) {
        Ok(integrate::Outcome::Integrated { into, how, pushed, removed, push_error, closed }) => {
            integrate::clear_attention(&w.path);
            let mut ev = removed_event(p, &w.path, &integ_branch, &into, &format!("integrated into {into} ({how})"), &closed);
            ev["kind"] = json!("integrated");
            ev["how"] = json!(how);
            ev["pushed"] = json!(pushed);
            ev["removed"] = json!(removed);
            push_event(ev);
            if let Some(c) = push_error {
                push_event(attention_event(p, &w.path, &integ_branch, &c));
            }
        }
        Ok(integrate::Outcome::Nothing) => {}
        Err(e) => {
            let why = integrate::as_coded(&e, &integ_branch);
            integrate::set_attention(&w.path, why.clone());
            push_event(attention_event(p, &w.path, &integ_branch, &why));
        }
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// The `End` a finished report describes.
pub fn end_of(r: &Result<axelstra_core::orch::Report, anyhow::Error>) -> End {
    match r {
        Ok(r) => End { clean: r.failed.is_empty() && r.needs_review.is_empty() && r.blocked.is_empty(), holds: !r.needs_review.is_empty() },
        Err(_) => End { clean: false, holds: false },
    }
}

/// True when the stored run state (`orch.status`) has nothing left to review, fail or wait for.
pub fn state_is_clean(v: &Value) -> bool {
    ["needs_review", "failed", "blocked"].iter().all(|k| v[k].as_array().is_none_or(|a| a.is_empty()))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fx {
        root: std::path::PathBuf,
        store: std::path::PathBuf,
        p: Project,
        wt_root: std::path::PathBuf,
    }

    impl Drop for Fx {
        fn drop(&mut self) {
            git::purge_trash(&self.wt_root);
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
        let root = std::env::temp_dir().join(format!("axl-oe-{tag}-{}", std::process::id()));
        let _ = git::remove_tree(&root);
        std::fs::create_dir_all(root.join("proj")).unwrap();
        let root = git::canon(&root).unwrap();
        let repo = root.join("proj");
        g(&repo, &["init", "-q", "-b", "main"]);
        std::fs::write(repo.join("base.txt"), "base\n").unwrap();
        g(&repo, &["add", "base.txt"]);
        g(&repo, &["commit", "-q", "-m", "base"]);
        let store = root.join("projects.json");
        let p = projects::add(&store, &repo.to_string_lossy()).unwrap();
        Fx { wt_root: root.join(".axelstra-wt"), root, store, p }
    }

    /// The integration worktree of run `name`, as the orchestrator makes it.
    fn integ(f: &Fx, name: &str) -> std::path::PathBuf {
        std::fs::create_dir_all(&f.wt_root).unwrap();
        let dir = f.wt_root.join(format!("{name}-integ"));
        git::worktree_add(Path::new(&f.p.path), &dir, &format!("orch/{name}"), "main").unwrap();
        dir
    }

    /// A merged task, like `finish_task`: commit on a task branch, `merge --no-ff` into the integration worktree.
    fn merge_task(f: &Fx, name: &str, file: &str, text: &str) {
        let repo = Path::new(&f.p.path);
        let dir = f.wt_root.join(format!("{name}-{file}"));
        git::worktree_add(repo, &dir, &format!("orch-task/{name}/{file}"), &format!("orch/{name}")).unwrap();
        std::fs::write(dir.join(file), text).unwrap();
        g(&dir, &["add", file]);
        g(&dir, &["commit", "-q", "-m", file]);
        let integ = f.wt_root.join(format!("{name}-integ"));
        assert!(matches!(git::merge(&integ, &format!("orch-task/{name}/{file}"), "merge").unwrap(), git::Merge::Clean));
        git::worktree_remove(repo, &dir).unwrap();
        git::branch_delete(repo, &format!("orch-task/{name}/{file}")).unwrap();
    }

    fn branch_exists(f: &Fx, b: &str) -> bool {
        git::run(Path::new(&f.p.path), &["rev-parse", "--verify", "--quiet", &format!("refs/heads/{b}")]).unwrap().ok
    }

    const CLEAN: End = End { clean: true, holds: false };

    fn project(f: &Fx) -> Project {
        projects::find(&f.store, &f.p.id).unwrap()
    }

    #[test]
    fn a_clean_run_is_integrated_and_its_worktree_and_branch_go() {
        let f = fx("ok");
        let dir = integ(&f, "ui-1");
        projects::set_auto(&f.store, &f.p.id, &dir.to_string_lossy()).unwrap();
        merge_task(&f, "ui-1", "a.txt", "a\n");
        finish(&project(&f), "ui-1", &CLEAN, &|_| vec![]);
        assert!(Path::new(&f.p.path).join("a.txt").exists(), "the work landed in main");
        assert!(!dir.exists());
        assert!(!branch_exists(&f, "orch/ui-1"));
    }

    #[test]
    fn a_conflict_keeps_the_work_with_the_reason_and_nothing_is_lost() {
        let f = fx("conflict");
        let repo = Path::new(&f.p.path);
        let dir = integ(&f, "ui-2");
        merge_task(&f, "ui-2", "base.txt", "theirs\n");
        std::fs::write(repo.join("base.txt"), "ours\n").unwrap();
        g(repo, &["commit", "-q", "-am", "ours"]);
        finish(&project(&f), "ui-2", &CLEAN, &|_| vec![]);
        assert!(dir.exists() && branch_exists(&f, "orch/ui-2"));
        assert_eq!(integrate::attention(&dir.to_string_lossy()).unwrap().code, "integrate.conflict");
        assert_eq!(std::fs::read_to_string(repo.join("base.txt")).unwrap().trim(), "ours");
    }

    #[test]
    fn an_empty_integration_worktree_goes_whatever_the_outcome() {
        for (tag, end) in [("e1", End { clean: false, holds: false }), ("e2", CLEAN)] {
            let f = fx(tag);
            let dir = integ(&f, "ui-3");
            finish(&project(&f), "ui-3", &end, &|_| vec![]);
            assert!(!dir.exists() && !branch_exists(&f, "orch/ui-3"), "{tag}");
        }
    }

    #[test]
    fn a_failed_run_with_work_keeps_it_and_says_why() {
        let f = fx("failed");
        let dir = integ(&f, "ui-4");
        merge_task(&f, "ui-4", "a.txt", "a\n");
        finish(&project(&f), "ui-4", &End { clean: false, holds: false }, &|_| vec![]);
        assert!(dir.exists() && branch_exists(&f, "orch/ui-4"));
        assert!(!Path::new(&f.p.path).join("a.txt").exists(), "a partial run is not integrated by itself");
        assert_eq!(integrate::attention(&dir.to_string_lossy()).unwrap().code, "orch.kept");
    }

    #[test]
    fn a_card_waiting_for_review_leaves_every_worktree_alone() {
        let f = fx("holds");
        let dir = integ(&f, "ui-5");
        finish(&project(&f), "ui-5", &End { clean: false, holds: true }, &|_| vec![]);
        assert!(dir.exists(), "orch.retry needs the integration worktree even when empty");
    }

    #[test]
    fn a_dirty_integration_worktree_is_never_removed() {
        let f = fx("dirty");
        let dir = integ(&f, "ui-6");
        std::fs::write(dir.join("wip.txt"), "w").unwrap();
        finish(&project(&f), "ui-6", &CLEAN, &|_| vec![]);
        assert!(dir.join("wip.txt").exists() && branch_exists(&f, "orch/ui-6"));
    }

    #[test]
    fn the_run_marks_its_worktrees_auto_and_the_janitor_cleans_an_integrated_one() {
        let f = fx("mark");
        mark_run(&f.store, &f.p.id, Path::new(&f.p.path), &f.wt_root, "ui-7");
        let p = project(&f);
        assert!(p.auto.contains(&axelstra_core::paths::norm(&f.wt_root.join("ui-7-integ").to_string_lossy())));
        let dir = integ(&f, "ui-7");
        merge_task(&f, "ui-7", "a.txt", "a\n");
        // integrated by hand (the Integrar button): the janitor takes the leftover
        g(Path::new(&f.p.path), &["merge", "-q", "--ff-only", "orch/ui-7"]);
        let out = janitor::sweep(&f.store, &|_| false, false);
        assert!(out.iter().any(|o| o["branch"] == "orch/ui-7" && o["removed"] == true), "{out:?}");
        assert!(!dir.exists() && !branch_exists(&f, "orch/ui-7"));
    }

    #[test]
    fn an_old_unmarked_orch_worktree_leaves_only_when_really_integrated() {
        let f = fx("legacy");
        let empty = integ(&f, "ui-8");
        let work = integ(&f, "ui-9");
        merge_task(&f, "ui-9", "a.txt", "a\n");
        let done = integ(&f, "ui-10");
        merge_task(&f, "ui-10", "b.txt", "b\n");
        projects::migrate_marks(&f.store).unwrap();
        let p = project(&f);
        assert!(p.auto.is_empty() && p.manual.len() == 3, "old ones are manual");
        // nothing is integrated yet: all stay, the empty one too
        assert!(janitor::sweep(&f.store, &|_| false, false).iter().all(|o| o["removed"] != true));
        g(Path::new(&f.p.path), &["merge", "-q", "--ff-only", "orch/ui-10"]);
        let out = janitor::sweep(&f.store, &|_| false, false);
        assert!(out.iter().any(|o| o["branch"] == "orch/ui-10" && o["removed"] == true), "{out:?}");
        assert!(!done.exists() && empty.exists() && work.exists());
    }

    #[test]
    fn a_leftover_task_branch_without_a_worktree_is_swept_when_empty_and_integrated_work_stays_safe() {
        let f = fx("branches");
        let repo = Path::new(&f.p.path);
        g(repo, &["branch", "orch-task/x/empty"]);
        g(repo, &["checkout", "-q", "-b", "orch/y"]);
        std::fs::write(repo.join("w.txt"), "w").unwrap();
        g(repo, &["add", "w.txt"]);
        g(repo, &["commit", "-q", "-m", "work"]);
        g(repo, &["checkout", "-q", "main"]);
        let mut out = vec![];
        crate::cleanup::stale_branches(&project(&f), false, &mut out);
        assert!(!branch_exists(&f, "orch-task/x/empty"), "{out:?}");
        assert!(branch_exists(&f, "orch/y"), "unintegrated work stays");
    }

    #[test]
    fn state_is_clean_reads_the_stored_run() {
        assert!(state_is_clean(&json!({"merged": ["a"], "failed": [], "needs_review": [], "blocked": []})));
        assert!(!state_is_clean(&json!({"merged": ["a"], "failed": [], "needs_review": [["b", "x"]], "blocked": []})));
        assert!(!state_is_clean(&json!({"failed": [["b", "x"]]})));
    }
}
