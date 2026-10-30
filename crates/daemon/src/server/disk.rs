//! Build artifacts of inactive worktrees and the free space of their volumes: the janitor's artifact pass, `disk.ensure`
//! (low disk: reclaim until above the line plus the margin), `disk.status` and `worktree.sweep {artifacts}`. Rules in
//! `crate::artifacts` and `crate::disk`. Events only: the janitor's wakes, `disk.ensure` (before a spawn, a worktree,
//! `axelstra lock`), the governor's cause tick and ONE deadline, the moment a worktree with artifacts becomes quiet.

use super::*;
use crate::artifacts::{self, Facts, OsTree, Tree};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// A worktree of a project, as the artifact pass sees it.
struct Wt {
    project: String,
    path: String,
    branch: String,
    main: bool,
    repo: PathBuf,
}

/// What the disk side of the registry remembers.
#[derive(Default)]
pub(super) struct DiskState {
    /// worktree (normalized) -> when a session last left it, ms since the epoch (`note_activity`); the janitor's own
    /// notes are dropped after 10 min, the quiet spell here is hours
    left: Mutex<HashMap<String, u64>>,
    /// the next moment a worktree with artifacts becomes quiet (the janitor's one artifact deadline)
    pub(super) due: Mutex<Option<Instant>>,
    /// a low-disk reclaim runs now
    reclaiming: AtomicBool,
    /// volumes the `disk.low` notice was raised for in this episode
    noticed: Mutex<HashSet<String>>,
    /// per volume: the janitor generation and free MB of the last low-disk attempt, so the cause tick does not try again
    /// until something changed (a session ended, a merge, or the volume lost another GB)
    tried: Mutex<HashMap<String, (u64, u64)>>,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// The folder lives where Axelstra puts worktrees: next to the repo, in `.axelstra-wt`.
fn in_wt_dir(path: &str) -> bool {
    path.replace('\\', "/").contains("/.axelstra-wt/")
}

impl DiskState {
    pub(super) fn note_left(&self, path: &str) {
        let mut m = self.left.lock_recover();
        let now = now_ms();
        m.insert(norm(path), now);
        let keep = artifacts::QUIET_HOURS_MAX * 3_600_000;
        m.retain(|_, at| now.saturating_sub(*at) < keep);
    }

    fn left_ms(&self, path: &str) -> Option<u64> {
        self.left.lock_recover().get(&norm(path)).copied()
    }

    /// How long until the artifact deadline (`None`: none set).
    pub(super) fn due_in(&self) -> Option<Duration> {
        self.due.lock_recover().map(|at| at.saturating_duration_since(Instant::now()).max(Duration::from_millis(20)))
    }
}

/// The earlier of two optional deadlines (the janitor's own and the artifact one).
pub(super) fn earliest(a: Option<Duration>, b: Option<Duration>) -> Option<Duration> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

/// The worktrees of every project (with the main checkouts, flagged), only those under `.axelstra-wt` besides the main.
fn worktrees(reg: &Registry) -> Vec<Wt> {
    let mut out = vec![];
    for p in crate::projects::load(&reg.store) {
        let repo = PathBuf::from(&p.path);
        for w in axelstra_core::git::worktree_list(&repo).unwrap_or_default() {
            if !w.is_main && !in_wt_dir(&w.path) {
                continue;
            }
            out.push(Wt { project: p.id.clone(), path: w.path, branch: w.branch.unwrap_or_default(), main: w.is_main, repo: repo.clone() });
        }
    }
    out
}

/// A build of `axelstra lock` (`mem.hold`) runs from a session of this worktree.
fn building_in(reg: &Registry, path: &str) -> bool {
    let want = norm(path);
    let sessions: Vec<u32> = reg.pauser.holds().into_iter().map(|(_, h)| h.session).collect();
    if sessions.is_empty() {
        return false;
    }
    let metas = reg.metas.lock_recover();
    let in_meta = sessions.iter().any(|id| metas.get(id).and_then(|m| m["worktree"].as_str()).is_some_and(|w| norm(w) == want));
    drop(metas);
    let specs = reg.specs.lock_recover();
    in_meta || sessions.iter().any(|id| specs.get(id).and_then(|s| s["cwd"].as_str()).is_some_and(|c| is_within(c, &want)))
}

/// A live or hibernated session, a session waiting to be restored, a flow, a run or a build uses the worktree.
fn in_use(reg: &Registry, path: &str) -> bool {
    !reg.live_in(path).is_empty()
        || reg.hibernated_in(path)
        || reg.worktree_pending(path)
        || reg.worktree_busy(path)
        || building_in(reg, path)
}

fn facts(reg: &Registry, t: &dyn Tree, w: &Wt, quiet_ms: u64, force: bool) -> Facts {
    let files = artifacts::files_used_ms(t, Path::new(&w.path));
    let last = [files, reg.disk.left_ms(&w.path)].into_iter().flatten().max();
    Facts { main: w.main, busy: !w.main && in_use(reg, &w.path), last_used_ms: last, now_ms: now_ms(), quiet_ms, force }
}

fn skipped(path: &str, e: &anyhow::Error) -> Value {
    match axelstra_core::msg::find(e) {
        Some(c) => json!({"path": path, "code": c.code, "params": c.params_json(), "text": c.text}),
        None => json!({"path": path, "code": Value::Null, "params": {}, "text": e.to_string()}),
    }
}

/// After a reclaim: the sizes are stale, the app hears it.
fn reclaimed_news(reg: &Registry) {
    crate::memo::memo().bump("disk");
    reg.broadcast("disk.changed");
    reg.announce_events();
}

fn reclaim_one(t: &dyn Tree, w: &Wt, dirs: &[(artifacts::Dir, u64)], out: &mut Vec<Value>, skip: &mut Vec<Value>) -> u64 {
    let r = artifacts::reclaim(t, dirs);
    if let Some((p, detail)) = &r.failed {
        let e = axelstra_core::coded!("artifacts.failed", "could not remove {path}: {detail}", path = p.display(), detail = detail);
        skip.push(skipped(&w.path, &e));
    }
    let freed = artifacts::mb(r.freed);
    if !r.removed.is_empty() {
        let list: Vec<Value> =
            r.removed.iter().map(|(d, n)| json!({"path": d.path, "kind": d.kind.as_str(), "mb": artifacts::mb(*n)})).collect();
        out.push(json!({"path": w.path, "dirs": list, "freed_mb": freed}));
        crate::janitor::push_event(json!({"kind": "artifacts", "project": w.project, "path": w.path, "branch": w.branch,
            "freed_mb": freed, "at": now_ms()}));
    }
    freed
}

/// One pass over the worktrees (or the one at `only`): reclaims the artifacts of every inactive one. `force` skips the
/// quiet rule only. `dry` measures what would go. Answer of `worktree.sweep {artifacts}`. The janitor's pass (`auto`)
/// does not list the worktrees in use and sets the next artifact deadline.
pub(super) fn pass(reg: &Registry, only: Option<&str>, force: bool, dry: bool, auto: bool) -> Value {
    let t = OsTree;
    let quiet = reg.with_settings(artifacts::quiet_ms);
    let (mut out, mut skip, mut freed) = (vec![], vec![], 0u64);
    let mut due: Option<u64> = None;
    // git lists canonical paths: the caller's spelling may be an alias (8.3 names on Windows, /var -> /private/var on macOS)
    let only = only.map(|o| axelstra_core::git::canon(Path::new(o)).map(|c| norm(&c.to_string_lossy())).unwrap_or_else(|_| norm(o)));
    for w in worktrees(reg) {
        if only.as_ref().is_some_and(|o| *o != norm(&w.path)) || (w.main && only.is_none()) {
            continue;
        }
        // nothing where artifacts live: nothing to say or wait for (no git run)
        if !w.main && artifacts::positions(&t, Path::new(&w.path)).is_empty() {
            continue;
        }
        let f = facts(reg, &t, &w, quiet, force);
        if let Err(e) = artifacts::check(&w.path, &f) {
            // the automatic pass keeps quiet about worktrees in use; the user's sweep hears why
            if let Some(d) = artifacts::due_in(&f).filter(|_| !f.busy) {
                due = Some(due.map_or(d, |x| x.min(d)));
            }
            if !auto || !f.busy {
                skip.push(skipped(&w.path, &e));
            }
            continue;
        }
        let dirs = artifacts::measure(&t, Path::new(&w.path));
        if dirs.is_empty() {
            continue;
        }
        if dry {
            let mb: u64 = dirs.iter().map(|(_, n)| artifacts::mb(*n)).sum();
            let list: Vec<Value> =
                dirs.iter().map(|(d, n)| json!({"path": d.path, "kind": d.kind.as_str(), "mb": artifacts::mb(*n)})).collect();
            out.push(json!({"path": w.path, "dirs": list, "freed_mb": mb}));
            freed += mb;
            continue;
        }
        freed += reclaim_one(&t, &w, &dirs, &mut out, &mut skip);
    }
    if auto {
        *reg.disk.due.lock_recover() = due.map(|d| Instant::now() + Duration::from_millis(d + 20));
    }
    if !dry && !out.is_empty() {
        reclaimed_news(reg);
    }
    json!({"artifacts": out, "skipped": skip, "freed_mb": freed})
}

/// The volumes holding the worktrees, with the line and whether each is low.
fn volumes(reg: &Registry, cfg: &crate::disk::Config) -> Vec<(crate::disk::Volume, String, bool)> {
    let dirs = crate::disk::wt_dirs(crate::projects::load(&reg.store).into_iter().map(|p| p.path));
    crate::disk::volumes(&dirs)
        .into_iter()
        .map(|(v, k)| {
            let low = cfg.is_low(v.free_mb, v.total_mb);
            (v, k, low)
        })
        .collect()
}

/// Reclaims on the low volumes until each is above its line plus the margin, oldest and biggest inactive worktrees first
/// (`disk::plan`). Still low afterwards: one `disk.low` notice per episode naming the biggest worktrees. Returns MB freed.
fn reclaim_low(reg: &Registry, cfg: &crate::disk::Config) -> u64 {
    let t = OsTree;
    let quiet = reg.with_settings(artifacts::quiet_ms);
    let mut freed = 0;
    let vols = volumes(reg, cfg);
    let all = worktrees(reg);
    for (v, key, low) in vols {
        if !low {
            continue;
        }
        reg.disk.tried.lock_recover().insert(key.clone(), (reg.janitor.now(), v.free_mb));
        let mine: Vec<&Wt> =
            all.iter().filter(|w| !w.main && crate::disk::key_of(Path::new(&w.path)).as_deref() == Some(key.as_str())).collect();
        let mut cands = vec![];
        let mut measured = vec![];
        for w in &mine {
            let f = facts(reg, &t, w, quiet, false);
            if artifacts::check(&w.path, &f).is_err() {
                continue;
            }
            let dirs = artifacts::measure(&t, Path::new(&w.path));
            let mb: u64 = dirs.iter().map(|(_, n)| artifacts::mb(*n)).sum();
            cands.push(crate::disk::Cand { path: w.path.clone(), mb, last_used_ms: f.last_used_ms.unwrap_or(0) });
            measured.push((*w, dirs));
        }
        let (mut out, mut skip) = (vec![], vec![]);
        for i in crate::disk::plan(&cands, cfg.need_mb(v.free_mb, v.total_mb)) {
            let (w, dirs) = &measured[i];
            freed += reclaim_one(&t, w, dirs, &mut out, &mut skip);
            // the real free space decides, not the sum of the sizes
            if crate::disk::free_space(Path::new(&w.path))
                .is_some_and(|(f, tot)| (f / (1 << 20)) > cfg.min_free_mb(tot / (1 << 20)) + cfg.margin_gb * 1024)
            {
                break;
            }
        }
        for s in &skip {
            eprintln!("disk: {s}");
        }
        let after = mine.first().and_then(|w| crate::disk::free_space(Path::new(&w.path))).map(|(f, tot)| (f >> 20, tot >> 20));
        let (free_mb, total_mb) = after.unwrap_or((v.free_mb, v.total_mb));
        if cfg.is_low(free_mb, total_mb) {
            if reg.disk.noticed.lock_recover().insert(key.clone()) {
                let rows: Vec<(String, u64)> = sizes(reg)["rows"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter(|r| r["volume"].as_str() == Some(key.as_str()))
                            .map(|r| {
                                let name = r["branch"].as_str().filter(|b| !b.is_empty()).or(r["path"].as_str()).unwrap_or_default();
                                (name.to_string(), r["total_mb"].as_u64().unwrap_or(0))
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let e = axelstra_core::coded!(
                    "disk.low",
                    "disk {volume} is low ({free_gb} GB free); biggest worktrees: {top}",
                    volume = v.root,
                    free_gb = free_mb / 1024,
                    top = crate::disk::top_text(&rows, 3)
                );
                if let Some(c) = axelstra_core::msg::find(&e) {
                    crate::store::notice_coded(c.clone());
                }
            }
        } else {
            reg.disk.noticed.lock_recover().remove(&key);
        }
    }
    if freed > 0 {
        reclaimed_news(reg);
    }
    freed
}

/// `disk.ensure`: reads the volumes; when one is low, reclaims (in a thread unless `wait`, so a spawn or a build is never
/// held up). `{low, free_mb, reclaimed_mb, reclaiming}`; `free_mb` is the least free among the volumes.
pub(super) fn ensure(reg: &Registry, wait: bool) -> Value {
    let cfg = reg.with_settings(crate::disk::Config::from_settings);
    let vols = volumes(reg, &cfg);
    let free = vols.iter().map(|(v, _, _)| v.free_mb).min();
    let low = vols.iter().any(|(_, _, l)| *l);
    if !low {
        reg.disk.noticed.lock_recover().clear();
        return json!({"low": false, "free_mb": free, "reclaimed_mb": 0, "reclaiming": false});
    }
    if reg.disk.reclaiming.swap(true, Ordering::SeqCst) {
        return json!({"low": true, "free_mb": free, "reclaimed_mb": 0, "reclaiming": true});
    }
    if wait {
        let n = reclaim_low(reg, &cfg);
        reg.disk.reclaiming.store(false, Ordering::SeqCst);
        let free = volumes(reg, &cfg).iter().map(|(v, _, _)| v.free_mb).min();
        let still = volumes(reg, &cfg).iter().any(|(_, _, l)| *l);
        return json!({"low": still, "free_mb": free, "reclaimed_mb": n, "reclaiming": false});
    }
    let Some(me) = reg.me.get().and_then(std::sync::Weak::upgrade) else {
        reg.disk.reclaiming.store(false, Ordering::SeqCst);
        return json!({"low": true, "free_mb": free, "reclaimed_mb": 0, "reclaiming": false});
    };
    std::thread::spawn(move || {
        let cfg = me.with_settings(crate::disk::Config::from_settings);
        reclaim_low(&me, &cfg);
        me.disk.reclaiming.store(false, Ordering::SeqCst);
    });
    json!({"low": true, "free_mb": free, "reclaimed_mb": 0, "reclaiming": true})
}

/// The governor's cause tick (it wakes anyway): a cheap free-space read; a low volume starts a reclaim, unless the last
/// attempt on it found nothing and nothing changed since (no janitor event, not a GB less free).
pub(super) fn tick(reg: &Registry) {
    let cfg = reg.with_settings(crate::disk::Config::from_settings);
    if !cfg.enabled {
        return;
    }
    let vols = volumes(reg, &cfg);
    let gen = reg.janitor.now();
    let tried = reg.disk.tried.lock_recover().clone();
    let fresh = vols.iter().any(|(v, k, low)| *low && tried.get(k).is_none_or(|(g, f)| *g != gen || v.free_mb + 1024 <= *f));
    if fresh {
        ensure(reg, false);
    } else if !vols.iter().any(|(_, _, l)| *l) {
        reg.disk.noticed.lock_recover().clear();
    }
}

/// Sizes of the worktrees, kept (`crate::memo`, scope `disk` and `wt`) until a reclaim, a `refresh` or a worktree added
/// or removed: measuring walks gigabytes, so it is never done on a timer. `{rows: [{project, path, branch, volume,
/// total_mb, target_mb, node_modules_mb, cache_mb, artifacts_mb}], measured_ms}`.
fn sizes(reg: &Registry) -> Value {
    let key = format!("{:p}|disk.sizes", reg as *const Registry);
    let opts = crate::memo::Opts { scopes: &["disk", "wt"], max_age: None, stale_ok: reg.under_pressure() };
    crate::memo::memo()
        .cached(&key, opts, || {
            let t = OsTree;
            let rows: Vec<Value> = worktrees(reg)
                .into_iter()
                .filter(|w| !w.main)
                .map(|w| {
                    let (dirs, total) = artifacts::measure_all(&t, Path::new(&w.path));
                    let of = |k: artifacts::Kind| dirs.iter().filter(|(d, _)| d.kind == k).map(|(_, n)| artifacts::mb(*n)).sum::<u64>();
                    json!({"project": w.project, "path": w.path, "branch": w.branch,
                        "volume": crate::disk::key_of(Path::new(&w.path)),
                        "total_mb": artifacts::mb(total), "target_mb": of(artifacts::Kind::Target),
                        "node_modules_mb": of(artifacts::Kind::NodeModules), "cache_mb": of(artifacts::Kind::Cache),
                        "artifacts_mb": dirs.iter().map(|(_, n)| artifacts::mb(*n)).sum::<u64>()})
                })
                .collect();
            reg.broadcast("disk.changed");
            Ok(json!({"rows": rows, "measured_ms": now_ms()}))
        })
        .unwrap_or_else(|_| json!({"rows": [], "measured_ms": Value::Null}))
}

/// `disk.status {refresh?}`.
pub(super) fn status(reg: &Registry, refresh: bool) -> Value {
    if refresh {
        crate::memo::memo().bump("disk");
    }
    let cfg = reg.with_settings(crate::disk::Config::from_settings);
    let quiet = reg.with_settings(artifacts::quiet_ms);
    let vols: Vec<Value> = volumes(reg, &cfg)
        .into_iter()
        .map(|(v, _, low)| json!({"root": v.root, "free_mb": v.free_mb, "total_mb": v.total_mb, "low": low, "min_free_mb": cfg.min_free_mb(v.total_mb)}))
        .collect();
    let s = sizes(reg);
    let t = OsTree;
    let all = worktrees(reg);
    let mut defaults: HashMap<PathBuf, (String, Vec<String>)> = HashMap::new();
    let mut rows = vec![];
    for r in s["rows"].as_array().cloned().unwrap_or_default() {
        let path = r["path"].as_str().unwrap_or_default().to_string();
        let Some(w) = all.iter().find(|w| norm(&w.path) == norm(&path)) else { continue };
        let (default, live) = defaults
            .entry(w.repo.clone())
            .or_insert_with(|| {
                let list = axelstra_core::git::worktree_list(&w.repo).unwrap_or_default();
                let live = list.iter().filter_map(|x| x.branch.clone()).filter(|b| crate::janitor::integrable_branch(b)).collect();
                (axelstra_core::git::default_branch(&w.repo), live)
            })
            .clone();
        let (integrated, ahead) = if w.branch.is_empty() {
            (false, 0)
        } else {
            let merged = crate::janitor::merged_reason_cached(&w.repo, &w.branch, &default, &live).is_some();
            (merged, crate::integrate::ahead_of(&w.repo, &w.branch, &default))
        };
        let f = facts(reg, &t, w, quiet, true);
        let sessions: Vec<u32> = reg.live_in(&w.path).into_iter().map(|(id, _)| id).collect();
        let art = r["artifacts_mb"].as_u64().unwrap_or(0);
        let mut row = r.clone();
        row["integrated"] = json!(integrated);
        row["ahead"] = json!(ahead);
        row["sessions"] = json!(sessions);
        row["active"] = json!(f.busy);
        row["last_used_ms"] = json!(f.last_used_ms);
        row["reclaimable_mb"] = json!(if f.busy { 0 } else { art });
        rows.push(row);
    }
    json!({"volumes": vols, "worktrees": rows, "measured_ms": s["measured_ms"], "disk": cfg.to_json(),
        "artifact_quiet_hours": quiet / 3_600_000})
}

pub(super) fn call(c: &mut Ctx) -> Result<Value> {
    let (p, reg) = (c.p, c.reg);
    match c.req.method.as_str() {
        "disk.status" => {
            let refresh = p["refresh"].as_bool() == Some(true);
            Ok(tokio::task::block_in_place(|| status(reg, refresh)))
        }
        "disk.ensure" => {
            let wait = p["wait"].as_bool() == Some(true);
            Ok(tokio::task::block_in_place(|| ensure(reg, wait)))
        }
        _ => Err(unknown_method(&c.req.method)),
    }
}

/// `worktree.sweep {artifacts: true, dry_run?, path?, force?}`.
pub(super) fn sweep(reg: &Registry, p: &Value) -> Value {
    let dry = p["dry_run"].as_bool().unwrap_or(false);
    let force = p["force"].as_bool().unwrap_or(false);
    let only = p["path"].as_str().filter(|s| !s.is_empty());
    tokio::task::block_in_place(|| pass(reg, only, force, dry, false))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_earliest_deadline_wins_and_none_is_no_deadline() {
        let s = Duration::from_secs;
        assert_eq!(earliest(None, None), None);
        assert_eq!(earliest(Some(s(5)), None), Some(s(5)));
        assert_eq!(earliest(None, Some(s(7))), Some(s(7)));
        assert_eq!(earliest(Some(s(9)), Some(s(7))), Some(s(7)));
    }

    #[test]
    fn only_folders_under_axelstra_wt_count() {
        assert!(in_wt_dir("C:\\x\\.axelstra-wt\\p-a"));
        assert!(in_wt_dir("/home/x/.axelstra-wt/p-a"));
        assert!(!in_wt_dir("/home/x/.axelstra-wt-fake/p-a"));
        assert!(!in_wt_dir("/home/x/p"));
    }

    #[test]
    fn the_left_note_outlives_the_janitor_note_and_the_deadline_is_read_back() {
        let d = DiskState::default();
        d.note_left("/a/.axelstra-wt/x");
        assert!(d.left_ms("/a/.axelstra-wt/x").is_some());
        assert_eq!(d.due_in(), None);
        *d.due.lock_recover() = Some(Instant::now() + Duration::from_secs(3600));
        let left = d.due_in().unwrap();
        assert!(left > Duration::from_secs(3590) && left <= Duration::from_secs(3600));
    }
}
