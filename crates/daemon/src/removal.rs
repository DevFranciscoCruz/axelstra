//! Full cleanup when the user removes a project (`project.remove` with `mode: "all"`): the worktrees Axelstra made for
//! it, their `ax/*` branches and everything the daemon keeps about the project. The project's own checkout is never
//! touched. Sessions are closed by the caller first (`server/removal.rs`); this module is the part that needs no
//! registry: which worktrees are ours to remove, the removal itself, the queue of removals that stayed pending (a
//! process outside our sessions still holds the folder) and the project's files in the data dir.

use crate::projects::Project;
use axelstra_core::paths::norm;
use crate::store::{read_json, write_atomic, write_json, LockRecover};
use anyhow::Result;
use axelstra_core::git;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Serializes read-modify-write of the pending queue (the project removal and the janitor both change it).
static QUEUE_LOCK: Mutex<()> = Mutex::new(());

/// A worktree removal that could not finish yet: the folder is held by a process we do not own.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Pending {
    /// the main checkout of the removed project (git runs here)
    pub repo: String,
    pub path: String,
    #[serde(default)]
    pub branch: Option<String>,
}

fn queue_path(store: &Path) -> PathBuf {
    store.with_file_name("removals.json")
}

/// The worktree folder lives where Axelstra puts them: next to the repo, in `.axelstra-wt`.
fn is_axelstra_dir(path: &str) -> bool {
    path.replace('\\', "/").contains("/.axelstra-wt/")
}

/// The worktrees of the project that a removal may take: listed by git, not the main checkout, inside `.axelstra-wt`.
/// This is the only door to deleting a folder: nothing outside this list is ever removed.
pub fn removable_worktrees(pr: &Project) -> Vec<git::Worktree> {
    let main = norm(&pr.path);
    git::worktree_list(Path::new(&pr.path))
        .unwrap_or_default()
        .into_iter()
        .filter(|w| !w.is_main && norm(&w.path) != main && is_axelstra_dir(&w.path))
        .collect()
}

/// What the preview says about one worktree.
pub struct Look {
    pub path: String,
    pub branch: Option<String>,
    /// uncommitted changes (or git cannot tell)
    pub dirty: bool,
    /// the `ax/*` branch has commits the default branch does not have
    pub unmerged: bool,
}

/// Looks at each removable worktree, read only.
pub fn preview(pr: &Project) -> Vec<Look> {
    let repo = Path::new(&pr.path);
    let default = git::default_branch(repo);
    removable_worktrees(pr)
        .into_iter()
        .map(|w| {
            let here = Path::new(&w.path).exists();
            let dirty = here && git::has_changes(Path::new(&w.path)).unwrap_or(true);
            let unmerged = w.branch.as_deref().is_some_and(|b| {
                crate::janitor::integrable_branch(b)
                    && crate::integrate::ahead_of(repo, b, &default) > 0
                    && !crate::cleanup::squash_merged(repo, b, &default)
            });
            Look { path: w.path, branch: w.branch, dirty, unmerged }
        })
        .collect()
}

fn branch_exists(repo: &Path, branch: &str) -> bool {
    git::run(repo, &["rev-parse", "--verify", "--quiet", &format!("refs/heads/{branch}")]).map(|o| o.ok).unwrap_or(false)
}

/// What one removal did.
#[derive(Debug, PartialEq)]
pub enum Gone {
    /// folder and git entry are gone; `branch` says whether an `ax/*` branch was deleted with them
    Removed { branch: bool },
    /// the folder is still held: nothing changed, it is queued
    Held,
}

/// Removes the worktree folder (long paths included) and git's entry, then its `ax/*` branch. All-or-nothing for the
/// folder (`git::worktree_remove_strict`), so a held folder leaves everything as it was. A branch outside `ax/*` stays.
pub fn remove_one(repo: &Path, path: &str, branch: Option<&str>) -> Gone {
    if git::worktree_remove_strict(repo, Path::new(path)).is_err() {
        return Gone::Held;
    }
    let mut deleted = false;
    if let Some(b) = branch.filter(|b| crate::janitor::managed_branch(b)) {
        let existed = branch_exists(repo, b);
        let _ = git::branch_delete(repo, b);
        deleted = existed && !branch_exists(repo, b);
    }
    Gone::Removed { branch: deleted }
}

/// Result of `remove_worktrees`.
#[derive(Debug, Default, PartialEq)]
pub struct Tally {
    pub removed: u32,
    pub pending: u32,
    pub branches: u32,
}

/// Removes every removable worktree of the project; the ones still held are queued for `retry`.
pub fn remove_worktrees(store: &Path, pr: &Project) -> Tally {
    let repo = Path::new(&pr.path);
    let mut t = Tally::default();
    let mut held = vec![];
    for w in removable_worktrees(pr) {
        match remove_one(repo, &w.path, w.branch.as_deref()) {
            Gone::Removed { branch } => {
                t.removed += 1;
                t.branches += u32::from(branch);
            }
            Gone::Held => {
                t.pending += 1;
                held.push(Pending { repo: pr.path.clone(), path: w.path, branch: w.branch });
            }
        }
    }
    // a folder git no longer lists may still be there (parked by an earlier removal)
    if let Some(dir) = pr.path_parent_wt() {
        git::purge_trash(&dir);
    }
    let _ = git::run(repo, &["worktree", "prune"]);
    if !held.is_empty() {
        queue(store, held);
    }
    t
}

impl Project {
    /// `<repo parent>/.axelstra-wt`, where this project's worktrees live.
    fn path_parent_wt(&self) -> Option<PathBuf> {
        Path::new(&self.path).parent().map(|p| p.join(".axelstra-wt")).filter(|d| d.is_dir())
    }
}

fn load_queue(store: &Path) -> Vec<Pending> {
    read_json(&queue_path(store)).ok().flatten().unwrap_or_default()
}

fn save_queue(store: &Path, q: &[Pending]) {
    let f = queue_path(store);
    if q.is_empty() {
        let _ = std::fs::remove_file(&f);
    } else if let Err(e) = write_json(&f, &q.to_vec()) {
        eprintln!("removals: could not save the pending queue: {e:#}");
    }
}

fn queue(store: &Path, items: Vec<Pending>) {
    let _g = QUEUE_LOCK.lock_recover();
    let mut q = load_queue(store);
    for i in items {
        if !q.iter().any(|x| norm(&x.path) == norm(&i.path)) {
            q.push(i);
        }
    }
    save_queue(store, &q);
}

/// Whether removals are waiting for their folder to be released.
pub fn has_pending(store: &Path) -> bool {
    !load_queue(store).is_empty()
}

/// Tries the queued removals again (what a session exit, a daemon start and the janitor's backoff trigger). `close`
/// closes the sessions still in a folder. Returns how many were finished and how many are still held.
pub fn retry(store: &Path, close: &dyn Fn(&str) -> Vec<String>) -> (u32, u32) {
    let _g = QUEUE_LOCK.lock_recover();
    let q = load_queue(store);
    if q.is_empty() {
        return (0, 0);
    }
    let (mut done, mut left) = (0, vec![]);
    for it in q {
        let repo = Path::new(&it.repo);
        // only a folder git still lists as a linked worktree under `.axelstra-wt`, or one that is already gone, is touched
        let listed = git::worktree_list(repo).ok().and_then(|l| l.into_iter().find(|w| norm(&w.path) == norm(&it.path)));
        let safe = match &listed {
            Some(w) => !w.is_main && is_axelstra_dir(&w.path) && norm(&it.path) != norm(&it.repo),
            None => !Path::new(&it.path).exists(),
        };
        if !safe {
            // not ours to remove any more (the repo went, or the path is something else): forget it
            continue;
        }
        let _ = close(&it.path);
        match remove_one(repo, &it.path, it.branch.as_deref()) {
            Gone::Removed { .. } => done += 1,
            Gone::Held => left.push(it),
        }
    }
    let n = left.len() as u32;
    save_queue(store, &left);
    (done, n)
}

/// Files of the data dir that only exist for this project: its task files (`notes/<id>/`). Anything that is not a direct
/// child of `notes/` is left alone. Returns whether something was removed.
pub fn remove_notes(data_dir: &Path, project_id: &str) -> bool {
    let root = crate::notes::root(data_dir);
    let dir = root.join(crate::notes::safe_name(project_id));
    if dir.parent() != Some(root.as_path()) || !dir.is_dir() {
        return false;
    }
    crate::notes::forget_project(&crate::notes::safe_name(project_id));
    git::remove_tree(&dir).is_ok()
}

/// Drops the dispatcher's notes (`dispatch.json`) of these worktrees, keyed by their lowercase path.
pub fn forget_dispatch_meta(data_dir: &Path, paths: &[String]) -> bool {
    let f = data_dir.join("dispatch.json");
    let norm = |s: &str| s.replace('\\', "/").trim_end_matches('/').to_ascii_lowercase();
    let Some(mut meta) = std::fs::read_to_string(&f).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()) else { return false };
    let Some(map) = meta.as_object_mut() else { return false };
    let before = map.len();
    for p in paths {
        map.remove(&norm(p));
    }
    if map.len() == before {
        return false;
    }
    write_atomic(&f, &serde_json::to_vec_pretty(&meta).unwrap_or_default()).is_ok()
}

/// Removes the project's entry from `triggers.json`.
pub fn forget_triggers(store: &Path, project_id: &str) -> bool {
    let f = crate::triggers::store_path(store);
    let _g = crate::triggers::STORE_LOCK.lock_recover();
    let mut s = crate::triggers::load(&f);
    s.remove(project_id).is_some() && crate::triggers::save(&f, &s).is_ok()
}

/// Removes the project's automations from `automations.json`; the caller holds the automations lock.
pub fn forget_automations(store: &Path, project_id: &str) -> bool {
    let f = crate::automations::store_path(store);
    let all = crate::automations::load(&f);
    let keep: Vec<_> = all.iter().filter(|a| a.project != project_id).cloned().collect();
    keep.len() != all.len() && crate::automations::save_all(&f, &keep).is_ok()
}

/// Drops what the app mirrored into `settings.json` for the project: a `ui` key that ends in `.<id>`, `:<id>` or
/// `/<id>`, and the entry of the id inside a `ui` value that is a JSON object (per-project preferences).
pub fn forget_ui(settings: &mut Value, project_id: &str) -> bool {
    let Some(ui) = settings["ui"].as_object_mut() else { return false };
    let mut changed = false;
    let suffixes = [format!(".{project_id}"), format!(":{project_id}"), format!("/{project_id}")];
    let before = ui.len();
    ui.retain(|k, _| !suffixes.iter().any(|s| k.ends_with(s)));
    changed |= ui.len() != before;
    for v in ui.values_mut() {
        let Some(mut inner) = v.as_str().and_then(|t| serde_json::from_str::<Value>(t).ok()).filter(Value::is_object) else { continue };
        if inner.as_object_mut().and_then(|o| o.remove(project_id)).is_some() {
            *v = json!(inner.to_string());
            changed = true;
        }
    }
    changed
}

/// Writes the settings back after `forget_ui` changed them.
pub fn save_settings(saved: &Path, settings: &Value) -> Result<()> {
    write_json(&saved.with_file_name("settings.json"), settings)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ui_entries_of_the_project_go_and_others_stay() {
        let mut s = json!({"ui": {
            "axelstra.projectPrefs": json!({"a": {"agent": "claude"}, "b": {"agent": "codex"}}).to_string(),
            "axelstra.lastWorktree.a": "x", "axelstra.lastWorktree.b": "y", "theme": "dark"}});
        assert!(forget_ui(&mut s, "a"));
        let ui = s["ui"].as_object().unwrap();
        assert!(!ui.contains_key("axelstra.lastWorktree.a") && ui.contains_key("axelstra.lastWorktree.b") && ui.contains_key("theme"));
        let prefs: Value = serde_json::from_str(ui["axelstra.projectPrefs"].as_str().unwrap()).unwrap();
        assert!(prefs.get("a").is_none() && prefs.get("b").is_some());
        assert!(!forget_ui(&mut s, "a"));
    }

    #[test]
    fn only_axelstra_worktree_folders_count() {
        assert!(is_axelstra_dir("C:\\Users\\x\\Projetos\\.axelstra-wt\\proj-feature"));
        assert!(!is_axelstra_dir("C:/Users/x/Projetos/proj"));
        assert!(!is_axelstra_dir("C:/Users/x/Projetos/proj/.axelstra-wt-fake"));
    }
}
