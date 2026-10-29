//! `axelstra mem` and `axelstra resume`: the memory governor from the terminal.

use anyhow::{bail, Result};
use axelstra_daemon::client::Client;
use serde_json::{json, Value};

/// Sessions listed in the human output.
const TOP: usize = 10;

pub fn status(mut c: Client, as_json: bool) -> Result<()> {
    let r = c.call("mem.status", json!({}))?;
    if as_json {
        println!("{}", serde_json::to_string_pretty(&r)?);
        return Ok(());
    }
    print!("{}", render(&r));
    Ok(())
}

fn render(r: &Value) -> String {
    use std::fmt::Write;
    let mut o = String::new();
    let n = |v: &Value| v.as_u64().unwrap_or(0);
    let _ = writeln!(o, "memory: {}  free {} MB of {} MB", r["level"].as_str().unwrap_or("unknown"), n(&r["free_mb"]), n(&r["total_mb"]));
    let t = &r["thresholds"];
    let _ = writeln!(
        o,
        "high at or below {}% or {} MB free, critical at or below {}% or {} MB (left only above +{}% and +{} MB)",
        n(&t["high_free_pct"]),
        n(&t["high_free_mb"]),
        n(&t["crit_free_pct"]),
        n(&t["crit_free_mb"]),
        n(&t["margin_pct"]),
        n(&t["margin_mb"])
    );
    let c = &r["config"];
    let on = |k: &str| if c[k].as_bool() == Some(true) { "on" } else { "off" };
    let _ = writeln!(
        o,
        "governor {}, admission {}, auto-pause {}, warn-only {}",
        on("enabled"),
        on("admission"),
        on("auto_pause"),
        on("warn_only")
    );
    let sessions = r["sessions"].as_array().cloned().unwrap_or_default();
    if !sessions.is_empty() {
        let _ = writeln!(o, "biggest sessions:");
        for s in sessions.iter().take(TOP) {
            let _ = writeln!(o, "  session {:<5} {} MB", n(&s["id"]), n(&s["rss_mb"]));
        }
    }
    let paused = r["paused"].as_array().cloned().unwrap_or_default();
    if paused.is_empty() {
        let _ = writeln!(o, "paused: none");
    } else {
        let _ = writeln!(o, "paused:");
        for p in &paused {
            let what = match p["kind"].as_str() {
                Some("build") => format!("build '{}' of session {}", p["name"].as_str().unwrap_or("?"), p["session"]),
                _ => format!("session {}", p["session"]),
            };
            let _ = writeln!(o, "  {what} ({}, {} processes)", p["reason"].as_str().unwrap_or("?"), n(&p["pids"]));
        }
    }
    o
}

pub fn resume(mut c: Client, session: Option<u32>, all: bool) -> Result<()> {
    let params = match (session, all) {
        (Some(_), true) => bail!("give a session or --all, not both"),
        (None, false) => bail!("give a session or --all"),
        (Some(id), false) => json!({"session": id}),
        (None, true) => json!({"all": true}),
    };
    let r = c.call("mem.resume", params)?;
    println!("resumed {}", r["count"].as_u64().unwrap_or(0));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_report_names_level_threshold_and_what_is_paused() {
        let r = json!({
            "level": "critical", "free_mb": 900, "total_mb": 16384,
            "thresholds": {"high_free_pct": 20, "crit_free_pct": 10, "high_free_mb": 3072, "crit_free_mb": 1536, "margin_pct": 5, "margin_mb": 512},
            "config": {"enabled": true, "admission": true, "auto_pause": true, "warn_only": false},
            "sessions": [{"id": 3, "rss_mb": 1200}],
            "paused": [{"kind": "build", "session": 3, "name": "verify", "reason": "memory", "pids": 4}],
        });
        let t = render(&r);
        assert!(t.contains("memory: critical  free 900 MB of 16384 MB"), "{t}");
        assert!(t.contains("session 3     1200 MB"), "{t}");
        assert!(t.contains("build 'verify' of session 3 (memory, 4 processes)"), "{t}");
    }
}
