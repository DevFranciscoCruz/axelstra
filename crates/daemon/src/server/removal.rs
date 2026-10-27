//! `project.remove_preview` and `project.remove`. Mode `list` only drops the project from the list (the old behaviour).
//! Mode `all` also closes its sessions, removes its worktrees and `ax/*` branches, and cleans the records the daemon
//! kept about it; the project's own checkout is never touched. See `crate::removal` for the parts without a registry.

use super::*;
use crate::projects::Project;
use std::collections::BTreeSet;

/// What belongs to the project among the sessions: live and registered ones, and the saved ones waiting to be restored.
struct Owned {
    ids: Vec<u32>,
    terminals: usize,
    pending: usize,
}

impl Registry {
    fn owns(&self, pr: &Project, paths: &[String], id: u32, metas: &HashMap<u32, Value>) -> bool {
        let m = metas.get(&id);
        let by_meta = m.is_some_and(|m| m["project"].as_str() == Some(pr.id.as_str()));
        by_meta || m.and_then(|m| m["worktree"].as_str()).is_some_and(|w| paths.iter().any(|p| norm(p) == norm(w)))
    }

    /// Records saved for the next start (not running now) that belong to the project.
    fn pending_of_project(&self, pr: &Project, paths: &[String]) -> Vec<Value> {
        let paths: Vec<String> = paths.iter().map(|p| norm(p)).collect();
        self.pending
            .lock_recover()
            .iter()
            .filter(|r| {
                r["meta"]["project"].as_str() == Some(pr.id.as_str())
                    || r["meta"]["worktree"].as_str().is_some_and(|w| paths.contains(&norm(w)))
                    || r["cwd"].as_str().is_some_and(|c| paths.iter().any(|p| is_within(c, p)))
            })
            .cloned()
            .collect()
    }

    /// Every session of the project: by its meta, or running in the main checkout or a worktree of it.
    fn project_sessions(&self, pr: &Project, wt_paths: &[String]) -> Owned {
        let mut paths = wt_paths.to_vec();
        paths.push(pr.path.clone());
        let mut ids: BTreeSet<u32> = BTreeSet::new();
        {
            let sessions: Vec<u32> = self.sessions.lock_recover().keys().copied().collect();
            let metas = self.metas.lock_recover();
            ids.extend(sessions.into_iter().filter(|id| self.owns(pr, &paths, *id, &metas)));
        }
        for p in &paths {
            ids.extend(self.live_in(p).into_iter().map(|(id, _)| id));
        }
        let metas = self.metas.lock_recover();
        let terminals = ids.iter().filter(|id| metas.get(id).is_some_and(|m| m["kind"] == "terminal")).count();
        Owned { ids: ids.into_iter().collect(), terminals, pending: self.pending_of_project(pr, &paths).len() }
    }

    /// Closes the project's sessions (the whole process tree of each) and forgets them; returns how many there were.
    fn close_project_sessions(&self, pr: &Project, wt_paths: &[String]) -> usize {
        let owned = self.project_sessions(pr, wt_paths);
        let live: Vec<Arc<Session>> = {
            let sessions = self.sessions.lock_recover();
            owned.ids.iter().filter_map(|id| sessions.get(id).cloned()).filter(|s| s.try_wait().is_none()).collect()
        };
        if !live.is_empty() {
            self.tear_down(&live);
        }
        for id in &owned.ids {
            self.forget_session(*id);
            self.stall_notified.lock_recover().remove(id);
        }
        // saved sessions waiting for a restore: they would bring the project's agents back
        let mut paths = wt_paths.to_vec();
        paths.push(pr.path.clone());
        let doomed = self.pending_of_project(pr, &paths);
        {
            let mut pending = self.pending.lock_recover();
            pending.retain(|r| !doomed.contains(r));
        }
        for r in &doomed {
            self.drop_sid_of(r);
        }
        owned.ids.len() + doomed.len()
    }

    /// Plans of running flows that use a worktree of the project.
    fn forget_project_flows(&self, paths: &[String]) {
        let paths: Vec<String> = paths.iter().map(|p| norm(p)).collect();
        self.flows.lock_recover().retain(|_, t| {
            !t.as_array().is_some_and(|a| a.iter().any(|x| x["worktree"].as_str().is_some_and(|w| paths.contains(&norm(w)))))
        });
    }

    /// Everything the daemon kept about the project besides its sessions and worktrees.
    fn forget_project_records(&self, pr: &Project, paths: &[String]) {
        let dir = self.saved.parent().unwrap_or(std::path::Path::new("")).to_path_buf();
        crate::removal::remove_notes(&dir, &pr.id);
        crate::removal::forget_dispatch_meta(&dir, paths);
        crate::removal::forget_triggers(&self.store, &pr.id);
        {
            let _g = self.auto_lock.lock_recover();
            if crate::removal::forget_automations(&self.store, &pr.id) {
                self.auto_wake.notify_one();
            }
        }
        let mut settings = self.settings();
        if crate::removal::forget_ui(&mut settings, &pr.id) {
            if let Err(e) = crate::removal::save_settings(&self.saved, &settings) {
                eprintln!("project removal: settings: {e:#}");
            }
            self.settings_changed();
        }
        for p in paths {
            crate::integrate::clear_attention(p);
            let _ = crate::integrate::done_mark(p, true);
            self.activity.lock_recover().remove(&axelstra_core::paths::norm(p));
        }
        crate::janitor::drop_events_of(&pr.id);
        self.forget_project_flows(paths);
    }
}

pub(super) fn call(c: &mut Ctx) -> Result<Value> {
    let (p, reg) = (c.p, c.reg);
    let pr = crate::projects::find(&reg.store, p["id"].as_str().unwrap_or_default())?;
    if c.req.method == "project.remove_preview" {
        let looks = tokio::task::block_in_place(|| crate::removal::preview(&pr));
        let paths: Vec<String> = looks.iter().map(|l| l.path.clone()).collect();
        let owned = reg.project_sessions(&pr, &paths);
        let wts: Vec<Value> = looks
            .iter()
            .map(|l| json!({"path": l.path, "branch": l.branch.clone().unwrap_or_default(), "dirty": l.dirty, "unmerged": l.unmerged}))
            .collect();
        let total = owned.ids.len() + owned.pending;
        return Ok(json!({"sessions": total - owned.terminals.min(total), "terminals": owned.terminals, "worktrees": wts}));
    }
    let mode = p["mode"].as_str().unwrap_or("list");
    if !["list", "all"].contains(&mode) {
        return Err(axelstra_core::coded!("project.remove_bad_mode", "mode must be list or all"));
    }
    if mode == "list" {
        crate::projects::remove(&reg.store, &pr.id)?;
        reg.projects_changed();
        return Ok(json!({"removed": true, "sessions_closed": 0, "worktrees_removed": 0, "worktrees_pending": 0, "branches_deleted": 0}));
    }
    let (closed, tally) = tokio::task::block_in_place(|| {
        let wt_paths: Vec<String> = crate::removal::removable_worktrees(&pr).into_iter().map(|w| w.path).collect();
        // sessions first: they hold their folders open (and run their processes) on Windows
        let closed = reg.close_project_sessions(&pr, &wt_paths);
        let tally = crate::removal::remove_worktrees(&reg.store, &pr);
        reg.forget_project_records(&pr, &wt_paths);
        (closed, tally)
    });
    // the project leaves the list last: if anything above had crashed the user could try again
    crate::projects::remove(&reg.store, &pr.id)?;
    reg.persist();
    reg.projects_changed();
    reg.pulse.bump();
    // removals still held are retried by the janitor (a session exit or its backoff wakes it), never by a fixed timer
    if tally.pending > 0 {
        reg.janitor.bump();
    }
    Ok(json!({"removed": true, "sessions_closed": closed, "worktrees_removed": tally.removed, "worktrees_pending": tally.pending,
              "branches_deleted": tally.branches}))
}
