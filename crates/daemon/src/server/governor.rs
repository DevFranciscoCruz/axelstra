//! `mem.*`: the machine's memory pressure (see `crate::governor`) and its settings, plus the thread that feeds it.
use super::*;
use crate::governor::{Change, Config, OnChange};

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
    reg.governor.start(crate::governor::os_source(), notifier(reg));
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
            }))
        }
        // the user freezes a session's process tree; only `mem.resume` (or a kill) lets it go
        "mem.pause" => {
            let id = p["session"].as_u64().and_then(|x| u32::try_from(x).ok()).ok_or_else(|| missing("session"))?;
            reg.get(id).filter(|s| s.try_wait().is_none()).ok_or_else(|| axelstra_core::coded!("session.not_found", "no such session {id}", id = id))?;
            let done = tokio::task::block_in_place(|| reg.pause_session(id, pause::MANUAL));
            if !done {
                return Err(axelstra_core::coded!("governor.pause_failed", "session {id} could not be paused (already paused, or nothing to stop)", id = id));
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
        "mem.admit" => {
            Ok(admit(reg, Duration::from_millis(p["wait_ms"].as_u64().unwrap_or(0).min(60_000))))
        }
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
}
