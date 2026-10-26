//! Worktree overview for the status bar: per project, how many worktrees there are and what state they are in
//! (uncommitted changes, ahead/behind the upstream, merged and safe to remove, no running agent). It runs git
//! once per worktree, so the answer is cached for a few seconds and computed by one caller at a time.

use crate::projects::{self, Project};
use crate::wtstatus;
use axelstra_core::git;
use serde::Serialize;
use serde_json::{json, Value};
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const TTL: Duration = Duration::from_secs(10);

/// One worktree as the overview sees it.
#[derive(Debug, Clone, Default)]
pub struct Row {
    pub path: String,
    pub branch: String,
    pub dirty: bool,
    pub ahead: bool,
    pub behind: bool,
    /// why the branch counts as merged (only set for clean worktrees nobody works in)
    pub merged: Option<String>,
    pub busy: bool,
    /// commits the default branch lacks, for `ax/*` branches nobody works in that are not merged ("por integrar")
    pub unintegrated: u32,
    /// why the last automatic integration did not happen (`integrate::attention`)
    pub attention: Option<axelstra_core::msg::Coded>,
}

#[derive(Serialize, Debug, PartialEq, Default)]
pub struct Counts {
    pub total: u32,
    pub dirty: u32,
    pub ahead: u32,
    pub behind: u32,
    pub merged: u32,
    /// worktrees with no running session in them
    pub no_agent: u32,
}

pub fn count(rows: &[Row]) -> Counts {
    let n = |f: &dyn Fn(&Row) -> bool| rows.iter().filter(|r| f(r)).count() as u32;
    Counts {
        total: rows.len() as u32,
        dirty: n(&|r| r.dirty),
        ahead: n(&|r| r.ahead),
        behind: n(&|r| r.behind),
        merged: n(&|r| r.merged.is_some()),
        no_agent: n(&|r| !r.busy),
    }
}

/// The rows of one project (the main checkout is not a worktree to clean up, so it is left out).
fn rows(p: &Project, busy: &dyn Fn(&str) -> bool) -> Vec<Row> {
    let repo = Path::new(&p.path);
    let default = git::default_branch(repo);
    let mut out = vec![];
    let list = git::worktree_list(repo).unwrap_or_default();
    // the live `ax/*` branches a stacked branch can be integrated into (same rule as the janitor)
    let live: Vec<String> = list.iter().filter_map(|w| w.branch.clone()).filter(|b| crate::janitor::integrable_branch(b)).collect();
    for w in list {
        if w.is_main {
            continue;
        }
        let st = wtstatus::status_cached(&w.path);
        let branch = w.branch.clone().unwrap_or_default();
        let busy = busy(&w.path);
        // the same rule as the janitor: only Axelstra's own branches, and never one with changes or a session
        let merged = if crate::janitor::integrable_branch(&branch) && st.dirty == 0 && !busy {
            crate::janitor::merged_reason_in(repo, &branch, &default, &live)
        } else {
            None
        };
        let pending = crate::janitor::integrable_branch(&branch) && !busy && merged.is_none();
        let unintegrated = if pending { crate::integrate::ahead_of(repo, &branch, &default) } else { 0 };
        let attention = if pending { crate::integrate::attention(&w.path) } else { None };
        out.push(Row {
            path: w.path,
            branch,
            dirty: st.dirty > 0,
            ahead: st.ahead > 0,
            behind: st.behind > 0,
            merged,
            busy,
            unintegrated,
            attention,
        });
    }
    out
}

fn project_json(p: &Project, rows: &[Row]) -> Value {
    let merged: Vec<Value> =
        rows.iter().filter_map(|r| r.merged.as_ref().map(|why| json!({"path": r.path, "branch": r.branch, "why": why}))).collect();
    // work nobody is doing and nothing integrated: the sidebar shows "por integrar" instead of an empty row
    let pending: Vec<Value> = rows
        .iter()
        .filter(|r| r.unintegrated > 0)
        .map(|r| {
            let mut v = json!({"path": r.path, "branch": r.branch, "ahead": r.unintegrated, "dirty": r.dirty});
            if let Some(a) = &r.attention {
                v["attention"] = json!({"code": a.code, "params": a.params_json(), "text": a.text});
            }
            v
        })
        .collect();
    json!({"id": p.id, "name": p.name, "counts": count(rows), "merged": merged, "pending": pending})
}

/// Uncached answer: `{projects: [{id, name, counts, merged: [{path, branch, why}], pending: [{path, branch, ahead, dirty, attention?}]}]}`.
pub fn compute(store: &Path, busy: &dyn Fn(&str) -> bool) -> Value {
    let list: Vec<Value> = projects::load(store).iter().map(|p| project_json(p, &rows(p, busy))).collect();
    json!({"projects": list})
}

static CACHE: Mutex<Option<(Instant, Value)>> = Mutex::new(None);
/// Held while computing, so a burst of polls costs one run of git and the others reuse its result.
static COMPUTING: Mutex<()> = Mutex::new(());

fn fresh() -> Option<Value> {
    CACHE.lock().ok()?.as_ref().filter(|(at, _)| at.elapsed() < TTL).map(|(_, v)| v.clone())
}

/// Forgets the cached answer (after something removed or created worktrees).
pub fn invalidate() {
    if let Ok(mut c) = CACHE.lock() {
        *c = None;
    }
}

/// `compute` reused for about 10 seconds. Call it from the blocking pool, never from a path others wait on.
pub fn cached(store: &Path, busy: &dyn Fn(&str) -> bool) -> Value {
    if let Some(v) = fresh() {
        return v;
    }
    let _g = COMPUTING.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(v) = fresh() {
        return v;
    }
    let v = compute(store, busy);
    if let Ok(mut c) = CACHE.lock() {
        *c = Some((Instant::now(), v.clone()));
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(f: impl Fn(&mut Row)) -> Row {
        let mut r = Row::default();
        f(&mut r);
        r
    }

    #[test]
    fn counts_each_state() {
        let rs = vec![
            row(|r| r.dirty = true),
            row(|r| {
                r.ahead = true;
                r.busy = true;
            }),
            row(|r| r.behind = true),
            row(|r| r.merged = Some("already in origin/main".into())),
        ];
        let c = count(&rs);
        assert_eq!((c.total, c.dirty, c.ahead, c.behind, c.merged, c.no_agent), (4, 1, 1, 1, 1, 3));
        assert_eq!(count(&[]), Counts::default());
    }

    #[test]
    fn project_json_lists_only_merged() {
        let p =
            Project { id: "p1".into(), name: "P".into(), path: ".".into(), setup: None, parents: Default::default(), ..Default::default() };
        let rs = vec![
            row(|r| r.branch = "ax/a".into()),
            row(|r| {
                r.branch = "ax/b".into();
                r.path = "/w/b".into();
                r.merged = Some("PR #3 merged".into());
            }),
        ];
        let v = project_json(&p, &rs);
        assert_eq!(v["counts"]["total"], 2);
        assert_eq!(v["merged"], json!([{"path": "/w/b", "branch": "ax/b", "why": "PR #3 merged"}]));
    }

    #[test]
    fn real_repo_fixture() {
        let dir = std::env::temp_dir().join(format!("axl-ovw-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let repo = dir.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let g = |cwd: &Path, args: &[&str]| assert!(git::run(cwd, args).unwrap().ok, "git {args:?}");
        let commit = ["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "--allow-empty", "-m", "x"];
        g(&repo, &["init", "-q", "-b", "main"]);
        g(&repo, &commit);
        g(&repo, &["update-ref", "refs/remotes/origin/main", "HEAD"]);
        let (done, dirty) = (dir.join("done"), dir.join("dirty"));
        for (wt, b) in [(&done, "ax/done"), (&dirty, "ax/dirty")] {
            g(&repo, &["worktree", "add", "-q", "-b", b, wt.to_str().unwrap()]);
            g(wt, &commit);
        }
        // `done` is merged into the default branch; `dirty` is not and has an untracked file
        g(&repo, &["merge", "-q", "--ff-only", "ax/done"]);
        g(&repo, &["update-ref", "refs/remotes/origin/main", "HEAD"]);
        std::fs::write(dirty.join("x.txt"), "x").unwrap();
        let p = Project {
            id: "p".into(),
            name: "P".into(),
            path: repo.to_string_lossy().into(),
            setup: None,
            parents: Default::default(),
            ..Default::default()
        };
        let rs = rows(&p, &|_| false);
        let c = count(&rs);
        assert_eq!((c.total, c.dirty, c.merged, c.no_agent), (2, 1, 1, 2), "{rs:?}");
        assert_eq!(rs.iter().filter(|r| r.merged.is_some()).map(|r| r.branch.as_str()).collect::<Vec<_>>(), ["ax/done"]);
        // a session in the merged worktree takes it off the list
        let rs = rows(&p, &|w| w.replace('\\', "/").ends_with("/done"));
        assert_eq!(count(&rs).merged, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
