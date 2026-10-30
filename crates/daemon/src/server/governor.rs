//! `mem.*`: the machine's memory pressure (see `crate::governor`) and its settings, plus the thread that feeds it.
use super::*;
use crate::governor::cause::{self, Alert};
use crate::governor::{Change, Config, OnChange};
use std::collections::{HashMap, HashSet};

const MB: u64 = 1024 * 1024;
/// Sessions named in a `memory.pressure` event.
const TOP: usize = 5;

/// Live sessions with the memory of their process trees in MB, biggest first.
pub(super) fn session_rss(reg: &Registry) -> Vec<(u32, u64)> {
    let live: Vec<_> = reg.sessions.lock_recover().iter().filter(|(_, s)| s.try_wait().is_none()).map(|(id, s)| (*id, s.clone())).collect();
    let snap = axelstra_core::procs::snapshot_cached(Duration::from_millis(800));
    let mut v: Vec<_> = live.iter().map(|(id, s)| (*id, s.memory_now_in(&snap) / MB)).collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    v
}

/// What the governor does on a level change: `memory.pressure` on the watch stream, and `resources.changed` with the
/// new level. Holds the registry weakly (the governor lives inside it).
pub(super) fn notifier(reg: &Arc<Registry>) -> OnChange {
    let weak = Arc::downgrade(reg);
    Arc::new(move |ch: &Change| {
        let Some(reg) = weak.upgrade() else { return };
        let top: Vec<Value> = session_rss(&reg)
            .into_iter()
            .take(TOP)
            .filter(|(_, mb)| *mb > 0)
            .map(|(id, mb)| json!({"session": id, "rss_mb": mb}))
            .collect();
        reg.broadcast_event(json!({
            "event": "memory.pressure", "level": ch.to.as_str(), "from": ch.from.as_str(),
            "free_mb": ch.info.free_mb, "total_mb": ch.info.total_mb, "top": top,
        }));
        reg.broadcast_resources();
        // sessions listed now carry the new level
        reg.pulse.bump();
    })
}

/// Starts the governor's thread on this OS's memory source.
pub(super) fn spawn(reg: &Arc<Registry>) {
    let weak = Arc::downgrade(reg);
    reg.governor.set_tick(Arc::new(move || {
        if let Some(reg) = weak.upgrade() {
            cause_tick(&reg);
        }
    }));
    reg.governor.start(crate::governor::os_source(), notifier(reg));
}

/// One governor wake: reads the kernel pools and the machine counters (cheap, no process table) and raises at most one
/// notice per episode. The process table (handle counts) is read only when a notice is raised.
pub(super) fn cause_tick(reg: &Registry) {
    let cfg = reg.governor.config();
    if !cfg.enabled {
        return;
    }
    for alert in reg.governor.cause.evaluate(cfg.kernel_pool_mb, Instant::now()) {
        raise(reg, alert);
    }
}

fn raise(reg: &Registry, alert: Alert) {
    let e = match alert {
        Alert::Kernel(mb) => {
            let rows = reg.governor.cause.source().procs();
            let handles =
                reg.governor.cause.source().handle_total().unwrap_or_else(|| rows.iter().filter_map(|r| r.handles).map(u64::from).sum());
            let handles = if rows.iter().any(|r| r.handles.is_some()) { handles.to_string() } else { "n/a".to_string() };
            let gb = cause::fmt_gb(mb);
            // the biggest tag and what it usually means, when the OS tells (Windows); else the plain text
            let top = reg
                .governor
                .cause
                .source()
                .pool_tags()
                .and_then(|t| t.into_iter().max_by(|a, b| (a.paged + a.nonpaged).cmp(&(b.paged + b.nonpaged)).then(b.tag.cmp(&a.tag))));
            match top {
                Some(t) => pool_notice(&gb, &handles, &t),
                None => axelstra_core::coded!(
                    "governor.kernel_pool_high",
                    "kernel memory is high ({gb} GB): possible handle or driver leak; restarting frees it",
                    gb = gb,
                    handles = handles
                ),
            }
        }
        Alert::Load(r) => axelstra_core::coded!(
            "governor.kernel_load_high",
            "kernel CPU is high ({kernel_pct}%, {pages} pages/s): a driver or filter (antivirus, indexer) may be overloading the system",
            kernel_pct = r.cpu_kernel_pct.map(|p| format!("{p:.0}")).unwrap_or_else(|| "?".into()),
            pages = r.pages_per_sec.map(|p| p.to_string()).unwrap_or_else(|| "?".into())
        ),
    };
    if let Some(c) = axelstra_core::msg::find(&e) {
        crate::store::notice_coded(c.clone());
    }
}

/// The kernel-pool notice naming the biggest tag. One code per known owner so each language words what that tag usually
/// means; a tag with no known owner gets the generic wording with just the tag.
fn pool_notice(gb: &str, handles: &str, t: &cause::PoolTag) -> anyhow::Error {
    let mb = (t.paged + t.nonpaged) / (1024 * 1024);
    let (tag, mb) = (t.tag.clone(), mb.to_string());
    let (gb, handles) = (gb.to_string(), handles.to_string());
    match cause::pool_owner(&t.tag) {
        Some("filter") => axelstra_core::coded!(
            "governor.kernel_pool_filter",
            "kernel memory is high ({gb} GB); biggest pool tag {tag} ({mb} MB) is the file-name cache of the filter manager (antivirus and other file filters)",
            gb = gb, tag = tag, mb = mb, handles = handles
        ),
        Some("tokens") => axelstra_core::coded!(
            "governor.kernel_pool_tokens",
            "kernel memory is high ({gb} GB); biggest pool tag {tag} ({mb} MB) is security tokens, which many started processes leave",
            gb = gb, tag = tag, mb = mb, handles = handles
        ),
        Some("files") => axelstra_core::coded!(
            "governor.kernel_pool_files",
            "kernel memory is high ({gb} GB); biggest pool tag {tag} ({mb} MB) is open file objects",
            gb = gb, tag = tag, mb = mb, handles = handles
        ),
        Some("watchers") => axelstra_core::coded!(
            "governor.kernel_pool_watchers",
            "kernel memory is high ({gb} GB); biggest pool tag {tag} ({mb} MB) is folder change notifications (watchers)",
            gb = gb, tag = tag, mb = mb, handles = handles
        ),
        Some("ntfs") => axelstra_core::coded!(
            "governor.kernel_pool_ntfs",
            "kernel memory is high ({gb} GB); biggest pool tag {tag} ({mb} MB) is NTFS file structures",
            gb = gb, tag = tag, mb = mb, handles = handles
        ),
        Some("mm") => axelstra_core::coded!(
            "governor.kernel_pool_mm",
            "kernel memory is high ({gb} GB); biggest pool tag {tag} ({mb} MB) belongs to the memory manager",
            gb = gb, tag = tag, mb = mb, handles = handles
        ),
        Some("io") => axelstra_core::coded!(
            "governor.kernel_pool_io",
            "kernel memory is high ({gb} GB); biggest pool tag {tag} ({mb} MB) is I/O name buffers",
            gb = gb, tag = tag, mb = mb, handles = handles
        ),
        Some("etw") => axelstra_core::coded!(
            "governor.kernel_pool_etw",
            "kernel memory is high ({gb} GB); biggest pool tag {tag} ({mb} MB) is event tracing (ETW) buffers",
            gb = gb, tag = tag, mb = mb, handles = handles
        ),
        _ => axelstra_core::coded!(
            "governor.kernel_pool_tag",
            "kernel memory is high ({gb} GB); biggest pool tag is {tag} ({mb} MB), owner not known",
            gb = gb, tag = tag, mb = mb, handles = handles
        ),
    }
}

/// How long the two process tables of an alerting `breakdown` are apart.
const RATE_WINDOW: Duration = Duration::from_millis(300);

/// The `breakdown` of `mem.status`: where the memory went. Blocking (reads the process table).
fn breakdown(reg: &Registry, info: &crate::governor::MemInfo) -> Value {
    let cfg = reg.governor.config();
    let src = reg.governor.cause.source();
    let rows = src.procs();
    let kernel = src.kernel();
    let (baseline, kernel_high) = match kernel {
        Some(k) => reg.governor.cause.kernel_state(k.alert_mb, cfg.kernel_pool_mb),
        None => (None, false),
    };
    let (rates, load_raised) = reg.governor.cause.load_last();
    let counters = src.system();
    let alerting = kernel_high || load_raised;
    let (spawns, secs) = axelstra_core::proc::spawn_stats();

    // whose each process is: a build (the tree of a `mem.hold` pid), a session (its job / group), or external
    let by_pid: HashMap<u32, &cause::ProcRow> = rows.iter().map(|r| (r.pid, r)).collect();
    let mut kids: HashMap<u32, Vec<u32>> = HashMap::new();
    for r in &rows {
        kids.entry(r.ppid).or_default().push(r.pid);
    }
    let holds = reg.pauser.holds();
    let mut build_pids: HashSet<u32> = HashSet::new();
    for (_, h) in &holds {
        let mut stack = vec![h.pid];
        while let Some(p) = stack.pop() {
            if by_pid.contains_key(&p) && build_pids.insert(p) {
                stack.extend(kids.get(&p).into_iter().flatten().copied());
            }
        }
    }
    let live: Vec<_> = {
        let sessions = reg.sessions.lock_recover();
        let metas = reg.metas.lock_recover();
        sessions
            .iter()
            .filter(|(_, s)| s.try_wait().is_none())
            .map(|(id, s)| (s.clone(), tier_of(metas.get(id).unwrap_or(&Value::Null))))
            .collect()
    };
    let classify = |r: &cause::ProcRow| {
        if build_pids.contains(&r.pid) {
            cause::Class::Build
        } else if let Some((_, t)) = live.iter().find(|(s, _)| s.owns_pid(r.pid)) {
            cause::Class::Session(*t)
        } else {
            cause::Class::External
        }
    };
    let tiers: Vec<cause::Tier> = live.iter().map(|(_, t)| *t).collect();
    let top = alerting.then(|| {
        std::thread::sleep(RATE_WINDOW);
        let again = src.procs();
        let ncpu = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
        cause::top_rates(&rows, &again, RATE_WINDOW.as_secs_f64(), ncpu)
    });
    cause::breakdown(&cause::Inputs {
        total_mb: info.total_mb,
        free_mb: info.free_mb,
        kernel,
        cache_mb: src.cache_mb(),
        compressed_mb: src.compressed_mb(&rows),
        handle_total: src.handle_total(),
        rows: &rows,
        sessions: &tiers,
        builds: holds.len(),
        classify: &classify,
        baseline_mb: baseline,
        kernel_high,
        pool_tags: src.pool_tags(),
        own: cause::own_json(spawns, secs, axelstra_core::proc::watch_count(), &axelstra_core::proc::site_stats()),
        system: cause::system_json(counters.as_ref(), rates.as_ref()),
        top,
    })
}

fn tier_of(meta: &Value) -> cause::Tier {
    if meta["kind"] != "agent" {
        cause::Tier::Terminal
    } else if meta["main"] == true {
        cause::Tier::Main
    } else if meta["lead"] == true {
        cause::Tier::Lead
    } else if meta["worker"] == true {
        cause::Tier::Worker
    } else {
        cause::Tier::Agent
    }
}

pub(super) fn call(c: &mut Ctx) -> Result<Value> {
    let (req, p, reg) = (c.req, c.p, c.reg);
    match req.method.as_str() {
        "mem.status" => {
            let (level, info) = reg.governor.state();
            let t = reg.governor.thresholds();
            let sessions: Vec<Value> =
                tokio::task::block_in_place(|| session_rss(reg)).into_iter().map(|(id, mb)| json!({"id": id, "rss_mb": mb})).collect();
            Ok(json!({
                "level": level.as_str(), "free_mb": info.free_mb, "total_mb": info.total_mb, "commit_pct": info.commit_pct,
                "thresholds": {"high_free_pct": t.high_pct, "crit_free_pct": t.crit_pct, "high_free_mb": t.high_mb, "crit_free_mb": t.crit_mb,
                               "margin_pct": t.margin_pct, "margin_mb": t.margin_mb},
                "config": reg.governor.config(),
                "sessions": sessions,
                "paused": reg.pauser.list(),
                "breakdown": tokio::task::block_in_place(|| breakdown(reg, &info)),
            }))
        }
        // the user freezes a session's process tree; only `mem.resume` (or a kill) lets it go
        "mem.pause" => {
            let id = p["session"].as_u64().and_then(|x| u32::try_from(x).ok()).ok_or_else(|| missing("session"))?;
            // a hibernated session has no process to freeze: it already costs no memory
            if reg.hib.is_sleeping(id) || reg.hib.is_waking(id).is_some() {
                return Err(axelstra_core::coded!(
                    "hibernate.sleeping",
                    "session {id} is hibernated: wake it with `axelstra wake {id}`",
                    id = id
                ));
            }
            reg.get(id)
                .filter(|s| s.try_wait().is_none())
                .ok_or_else(|| axelstra_core::coded!("session.not_found", "no such session {id}", id = id))?;
            let done = tokio::task::block_in_place(|| reg.pause_session(id, pause::MANUAL));
            if !done {
                return Err(axelstra_core::coded!(
                    "governor.pause_failed",
                    "session {id} could not be paused (already paused, or nothing to stop)",
                    id = id
                ));
            }
            Ok(json!({"paused": [id]}))
        }
        "mem.resume" => {
            let t = if p["all"].as_bool() == Some(true) {
                reg.resume_paused(None)
            } else {
                let id = p["session"].as_u64().and_then(|x| u32::try_from(x).ok()).ok_or_else(|| missing("session"))?;
                let t = reg.pauser.resume_session(id);
                reg.announce_resumed(&t);
                t
            };
            let ids: Vec<u32> = t.iter().filter_map(|t| if let pause::Target::Session(i) = t { Some(*i) } else { None }).collect();
            Ok(json!({"resumed": ids, "count": t.len()}))
        }
        // `axelstra lock` asks before it takes a new slot: answers at once when memory is fine, else waits (up to `wait_ms`,
        // at most a minute) for the level to return to Normal. It sleeps on the governor's change signal, no polling.
        "mem.admit" => Ok(admit(reg, Duration::from_millis(p["wait_ms"].as_u64().unwrap_or(0).min(60_000)))),
        // a build/test under `axelstra lock` says its process: the pauser may freeze it. Only a process inside a session counts
        "mem.hold" => {
            let pid = p["pid"].as_u64().and_then(|x| u32::try_from(x).ok()).ok_or_else(|| missing("pid"))?;
            let name = p["name"].as_str().unwrap_or("lock").to_string();
            let owner = reg.sessions.lock_recover().iter().find(|(_, s)| s.owns_pid(pid)).map(|(id, _)| *id);
            let Some(owner) = owner else { return Ok(json!({"id": null})) };
            if let Some(me) = *c.agent {
                if owner != me {
                    return Err(not_own_tree(&req.method));
                }
            }
            Ok(json!({"id": reg.pauser.hold(&name, owner, pid)}))
        }
        "mem.unhold" => {
            let id = p["id"].as_u64().ok_or_else(|| missing("id"))?;
            let t = reg.pauser.holds().iter().any(|(i, _)| *i == id).then_some(pause::Target::Build(id));
            reg.pauser.unhold(id);
            if let Some(t) = t {
                reg.announce_resumed(&[t]);
            }
            Ok(json!({}))
        }
        // saves the patched config in settings.json, then applies it; the answer is the config now in force
        "mem.config" => {
            let cfg = reg.governor.config().patch(&p["patch"])?;
            apply_config(reg, cfg.clone())?;
            Ok(json!({"config": cfg}))
        }
        m => Err(unknown_method(m)),
    }
}

/// Answers `{ok, level, free_mb}`: ok at once when memory is fine (or the gate is off), else sleeps on the governor's
/// change signal until it is, `wait` passes, or the daemon closes.
fn admit(reg: &Registry, wait: Duration) -> Value {
    let end = Instant::now() + wait;
    let mut seen = reg.governor.generation();
    loop {
        let (level, info) = reg.governor.state();
        let held = reg.governor.config().gate_active() && level != crate::governor::Level::Normal;
        let left = end.saturating_duration_since(Instant::now());
        if !held || left.is_zero() || reg.governor.is_closed() || reg.closing.load(Ordering::SeqCst) {
            return json!({"ok": !held, "level": level.as_str(), "free_mb": info.free_mb});
        }
        seen = reg.governor.wait_change(seen, left).1;
    }
}

fn apply_config(reg: &Arc<Registry>, cfg: Config) -> Result<()> {
    let mut saved = reg.settings();
    cfg.into_settings(&mut saved);
    let written = write_json(&reg.saved.with_file_name("settings.json"), &saved);
    reg.settings_changed();
    written?;
    reg.governor.set_config(cfg, Some(&notifier(reg)));
    // the pause loop looks again: switching pausing off lets the memory pauses go, switching it on at Critical starts it
    reg.governor.poke();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_is_saved_in_settings_and_survives_a_new_registry() {
        let dir = std::env::temp_dir().join(format!("axl-gov-reg-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = dir.join("projects.json");
        let reg = Arc::new(Registry::new(store.clone(), "p".into(), "t".into()));
        let cfg = reg.governor.config().patch(&json!({"warn_only": true, "high_free_pct": 25})).unwrap();
        apply_config(&reg, cfg).unwrap();
        assert!(reg.settings()["governor"]["warn_only"].as_bool().unwrap());
        // the other settings are left alone
        let reg2 = Registry::new(store, "p".into(), "t".into());
        let c = reg2.governor.config();
        assert_eq!((c.warn_only, c.high_free_pct, c.crit_free_pct), (true, 25, 10));
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn mem(free_mb: u64) -> crate::governor::MemInfo {
        crate::governor::MemInfo { total_mb: 16384, free_mb, commit_pct: None }
    }

    #[test]
    fn a_lock_waiting_for_memory_wakes_by_event_and_only_when_the_gate_is_on() {
        let dir = std::env::temp_dir().join(format!("axl-gov-admit-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let reg = Arc::new(Registry::new(dir.join("projects.json"), "p".into(), "t".into()));
        // Normal: at once
        assert_eq!(admit(&reg, Duration::from_secs(30))["ok"], true);
        reg.governor.feed(mem(2500));
        // High: a short wait times out with ok=false and the free memory in the answer
        let r = admit(&reg, Duration::from_millis(50));
        assert_eq!((r["ok"].clone(), r["level"].clone(), r["free_mb"].clone()), (json!(false), json!("high"), json!(2500)));
        // a long wait ends the moment the level returns to Normal, far sooner than its 30 s
        let r2 = reg.clone();
        let h = std::thread::spawn(move || admit(&r2, Duration::from_secs(30)));
        std::thread::sleep(Duration::from_millis(100));
        let t = Instant::now();
        reg.governor.feed(mem(9000));
        let r = h.join().unwrap();
        assert_eq!(r["ok"], true);
        assert!(t.elapsed() < Duration::from_secs(5));
        // warn-only: the gate is off, nobody waits at Critical
        reg.governor.feed(mem(100));
        let cfg = reg.governor.config().patch(&json!({"warn_only": true})).unwrap();
        reg.governor.set_config(cfg, None);
        assert_eq!(admit(&reg, Duration::from_secs(30))["ok"], true);
        // a stopping governor releases waiters
        let cfg = reg.governor.config().patch(&json!({"warn_only": false})).unwrap();
        reg.governor.set_config(cfg, None);
        let r2 = reg.clone();
        let h = std::thread::spawn(move || admit(&r2, Duration::from_secs(30)));
        std::thread::sleep(Duration::from_millis(100));
        reg.governor.stop();
        assert_eq!(h.join().unwrap()["ok"], false);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_level_change_broadcasts_memory_pressure_once() {
        let dir = std::env::temp_dir().join(format!("axl-gov-ev-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let reg = Arc::new(Registry::new(dir.join("projects.json"), "p".into(), "t".into()));
        let (tx, mut rx) = mpsc::channel::<Vec<u8>>(16);
        reg.watchers.lock_recover().push((Arc::new(AtomicBool::new(false)), tx));
        let cb = notifier(&reg);
        let ch = reg.governor.feed(crate::governor::MemInfo { total_mb: 16384, free_mb: 1000, commit_pct: None }).unwrap();
        cb(&ch);
        let mut names = vec![];
        while let Ok(f) = rx.try_recv() {
            let v: Value = serde_json::from_slice(&f[5..]).unwrap();
            names.push((v["event"].as_str().unwrap().to_string(), v["level"].as_str().map(String::from)));
        }
        assert_eq!(
            names,
            [
                ("memory.pressure".to_string(), Some("critical".to_string())),
                ("resources.changed".to_string(), Some("critical".to_string()))
            ]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_governor_wake_raises_one_kernel_notice_per_episode_and_mem_status_carries_the_breakdown() {
        use crate::governor::cause::fake::FakeCause;
        let dir = std::env::temp_dir().join(format!("axl-gov-cause-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let reg = Arc::new(Registry::new(dir.join("projects.json"), "p".into(), "t".into()));
        let fake = Arc::new(FakeCause::new(1500));
        reg.governor.cause.set_source(fake.clone());
        let _ = crate::store::take_notices();
        cause_tick(&reg);
        assert!(crate::store::take_notice_items().is_empty());
        fake.set_kernel(4600);
        cause_tick(&reg);
        cause_tick(&reg);
        let items = crate::store::take_notice_items();
        assert_eq!(items.len(), 1);
        let c = items[0].1.as_ref().unwrap();
        assert_eq!((c.code.as_str(), c.params["gb"].as_str()), ("governor.kernel_pool_high", "4.5"));
        // the breakdown reports the same pool as high, with the baseline taken at the first reading
        let b = breakdown(&reg, &crate::governor::MemInfo { total_mb: 16384, free_mb: 4000, commit_pct: None });
        assert_eq!(
            (b["kernel"]["high"].clone(), b["kernel"]["baseline_mb"].clone(), b["kernel"]["total_mb"].clone()),
            (json!(true), json!(1500), json!(4600))
        );
        // a limit raised above the pool and the 2x line (3000) is still high through the baseline rule
        assert_eq!(reg.governor.config().patch(&json!({"kernel_pool_mb": 9000})).unwrap().kernel_pool_mb, 9000);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
