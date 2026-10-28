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
        let top: Vec<Value> = session_rss(&reg).into_iter().take(TOP).filter(|(_, mb)| *mb > 0).map(|(id, mb)| json!({"session": id, "rss_mb": mb})).collect();
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
            }))
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

fn apply_config(reg: &Arc<Registry>, cfg: Config) -> Result<()> {
    let mut saved = reg.settings();
    cfg.into_settings(&mut saved);
    let written = write_json(&reg.saved.with_file_name("settings.json"), &saved);
    reg.settings_changed();
    written?;
    reg.governor.set_config(cfg, Some(&notifier(reg)));
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
        assert_eq!(names, [("memory.pressure".to_string(), Some("critical".to_string())), ("resources.changed".to_string(), Some("critical".to_string()))]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
