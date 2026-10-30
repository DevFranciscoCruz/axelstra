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
    breakdown(&mut o, &r["breakdown"], n(&r["total_mb"]));
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

/// Width of a bar in the breakdown.
const BAR: usize = 20;

fn bar(pct: f64) -> String {
    let full = ((pct / 100.0) * BAR as f64).round().clamp(0.0, BAR as f64) as usize;
    format!("[{}{}]", "#".repeat(full), ".".repeat(BAR - full))
}

fn gb(mb: u64) -> String {
    format!("{:.1}", mb as f64 / 1024.0)
}

/// The `breakdown` of `mem.status` (additive: a daemon without it prints nothing here). Absent numbers print nothing.
fn breakdown(o: &mut String, b: &Value, total: u64) {
    use std::fmt::Write;
    if !b.is_object() {
        return;
    }
    let n = |v: &Value| v.as_u64().unwrap_or(0);
    let f = |v: &Value| v.as_f64();
    let line = |o: &mut String, name: &str, mb: u64, extra: &str| {
        let pct = if total > 0 { mb as f64 * 100.0 / total as f64 } else { 0.0 };
        let _ = writeln!(o, "  {:<24} {} {:>7} MB {:>5.1}%{}", name, bar(pct), mb, pct, extra);
    };
    let _ = writeln!(o, "where the memory goes:");
    if let Some(sys) = b["system"].as_object() {
        let sys = Value::Object(sys.clone());
        if let (Some(u), Some(k)) = (f(&sys["cpu_user_pct"]), f(&sys["cpu_kernel_pct"])) {
            let _ = writeln!(o, "  CPU: user {u:.0}% kernel {k:.0}%");
        }
        if let (Some(c), Some(l)) = (sys["commit_mb"].as_u64(), sys["commit_limit_mb"].as_u64()) {
            let _ = writeln!(o, "  commit {} of {} GB", gb(c), gb(l));
        }
    }
    for s in b["sessions"].as_array().into_iter().flatten() {
        let extra = format!("  ({})", n(&s["count"]));
        line(o, &format!("sessions: {}", s["tier"].as_str().unwrap_or("?")), n(&s["rss_mb"]), &extra);
    }
    if n(&b["builds"]["count"]) > 0 || n(&b["builds"]["rss_mb"]) > 0 {
        let extra = format!("  ({})", n(&b["builds"]["count"]));
        line(o, "builds (axelstra lock)", n(&b["builds"]["rss_mb"]), &extra);
    }
    if b["kernel"].is_object() {
        let k = &b["kernel"];
        let base = k["baseline_mb"].as_u64().map(|v| format!("  (baseline {v} MB)")).unwrap_or_default();
        line(o, "kernel paged", n(&k["paged_mb"]), "");
        line(o, "kernel non-paged", n(&k["nonpaged_mb"]), &base);
    }
    pool_tags(o, &b["pool_tags"], &b["own"]);
    if let Some(v) = b["cache_mb"].as_u64() {
        line(o, "system cache", v, "");
    }
    if let Some(v) = b["compressed_mb"].as_u64() {
        line(o, "compressed memory", v, "");
    }
    line(o, "others", n(&b["others_mb"]), "");
    let ext = b["external"].as_array().cloned().unwrap_or_default();
    if !ext.is_empty() {
        let _ = writeln!(o, "programs outside Axelstra (information only):");
        for e in &ext {
            let c = n(&e["count"]);
            let times = if c > 1 { format!(" x{c}") } else { String::new() };
            let _ = writeln!(o, "  {:<24} {} MB", format!("{}{times}", e["name"].as_str().unwrap_or("?")), n(&e["rss_mb"]));
        }
    }
    for (key, head, unit) in [("top_cpu", "top CPU:", "%"), ("top_io", "top disk I/O:", "")] {
        let rows = b[key].as_array().cloned().unwrap_or_default();
        if rows.is_empty() {
            continue;
        }
        let _ = writeln!(o, "{head}");
        for r in &rows {
            let v = f(&r["value"]).unwrap_or(0.0);
            let _ = writeln!(o, "  {} ({}) {v:.0}{unit}", r["name"].as_str().unwrap_or("?"), n(&r["pid"]));
        }
    }
    let h = &b["handles"];
    if h.is_object() {
        let _ = writeln!(o, "handles: {} in total", n(&h["total"]));
        for e in h["top"].as_array().into_iter().flatten() {
            let _ = writeln!(o, "  {} ({}) {}", e["name"].as_str().unwrap_or("?"), n(&e["pid"]), n(&e["handles"]));
        }
    }
    if b["kernel"]["high"].as_bool() == Some(true) {
        let g = gb(n(&b["kernel"]["total_mb"]));
        let big = b["pool_tags"]["top"][0]["tag"].as_str();
        let why = match big {
            Some(t) => {
                let owner = b["pool_tags"]["top"][0]["owner"].as_str().map(|w| format!(", {}", owner_text(w))).unwrap_or_default();
                format!("; biggest pool tag {t}{owner}")
            }
            None => String::new(),
        };
        let _ = writeln!(o, "suggestion: kernel memory is high ({g} GB): possible handle or driver leak; restarting frees it{why}");
    }
}

/// Kernel pool per tag (Windows) and what Axelstra itself adds. Partial on purpose: the line "no tag" says how much is not named.
fn pool_tags(o: &mut String, p: &Value, own: &Value) {
    use std::fmt::Write;
    let n = |v: &Value| v.as_u64().unwrap_or(0);
    let f = |v: &Value| v.as_f64();
    if p.is_object() {
        let _ = writeln!(o, "kernel pool by tag (partial attribution):");
        for t in p["top"].as_array().into_iter().flatten() {
            let tag = t["tag"].as_str().unwrap_or("?");
            let owner = t["owner"].as_str().map(|w| format!("  {}", owner_text(w))).unwrap_or_default();
            let _ = writeln!(o, "  {tag:<5} paged {:>5} MB  non-paged {:>5} MB{owner}", n(&t["paged_mb"]), n(&t["nonpaged_mb"]));
        }
        if let Some(v) = p["sem_etiqueta_mb"].as_u64() {
            let _ = writeln!(o, "  no tag (large allocations, special pools): {v} MB");
        }
    }
    if own.is_object() {
        let rate = f(&own["spawns_per_min"]).map(|r| format!(", {r:.1}/min")).unwrap_or_default();
        let _ =
            writeln!(o, "axelstra itself: {} child processes started{rate}, {} directories watched", n(&own["spawns"]), n(&own["watches"]));
        if let Some(last) = own["spawns_last_min"].as_u64() {
            let _ = writeln!(o, "  last minute: {last} child processes; by site (last minute / total):");
            for s in own["sites"].as_array().into_iter().flatten() {
                let (site, cmd) = (s["site"].as_str().unwrap_or("?"), s["command"].as_str().unwrap_or("?"));
                let _ = writeln!(o, "  {:>6} {:>8}  {site}: {cmd}", n(&s["last_min"]), n(&s["total"]));
            }
        }
    }
}

/// English text of a pool tag owner id (the app words it in its own language).
fn owner_text(owner: &str) -> &'static str {
    match owner {
        "filter" => "file-name cache of the filter manager (antivirus and other file filters)",
        "tokens" => "security tokens (many processes started)",
        "files" => "open file objects",
        "watchers" => "folder change notifications (watchers)",
        "ntfs" => "NTFS file structures",
        "mm" => "memory manager",
        "io" => "I/O name buffers",
        "etw" => "event tracing (ETW) buffers",
        _ => "",
    }
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

    fn sample() -> Value {
        json!({
            "level": "normal", "free_mb": 8000, "total_mb": 16000,
            "thresholds": {}, "config": {}, "sessions": [], "paused": [],
            "breakdown": {
                "kernel": {"paged_mb": 300, "nonpaged_mb": 4000, "total_mb": 4300, "baseline_mb": 800, "high": true},
                "cache_mb": 1000, "compressed_mb": null,
                "sessions": [{"tier": "agent", "count": 2, "rss_mb": 1600}],
                "builds": {"count": 1, "rss_mb": 2000},
                "external": [{"name": "MsMpEng.exe", "count": 1, "rss_mb": 700}, {"name": "chrome.exe", "count": 12, "rss_mb": 3000}],
                "others_mb": 500,
                "pool_tags": {"top": [
                    {"tag": "FMfn", "paged_mb": 1541, "nonpaged_mb": 0, "owner": "filter"},
                    {"tag": "Xyz1", "paged_mb": 3, "nonpaged_mb": 2, "owner": null}],
                    "tagged_mb": 1546, "sem_etiqueta_mb": 1300, "partial": true},
                "own": {"spawns": 240, "uptime_secs": 120, "spawns_per_min": 120.0, "watches": 35, "spawns_last_min": 90,
                    "sites": [{"site": "dispatch.overlap_files", "command": "git diff", "total": 200, "last_min": 80}]},
                "handles": {"total": 300000, "top": [{"name": "leak.exe", "pid": 7, "handles": 90000}]},
                "system": {"cpu_user_pct": 12.0, "cpu_kernel_pct": 30.0, "commit_mb": 20480, "commit_limit_mb": 40960},
                "top_cpu": [{"name": "a.exe", "pid": 1, "value": 55.0}],
                "top_io": [{"name": "b.exe", "pid": 2, "value": 120.0}],
            },
        })
    }

    #[test]
    fn the_breakdown_lists_categories_externals_handles_and_the_suggestion() {
        let t = render(&sample());
        assert!(t.contains("where the memory goes:"), "{t}");
        assert!(t.contains("last minute: 90 child processes"), "{t}");
        assert!(t.contains("dispatch.overlap_files: git diff"), "{t}");
        assert!(t.contains("kernel non-paged") && t.contains("4000 MB  25.0%"), "{t}");
        assert!(t.contains("(baseline 800 MB)"), "{t}");
        assert!(t.contains("chrome.exe x12"), "{t}");
        assert!(t.contains("MsMpEng.exe"), "{t}");
        assert!(t.contains("CPU: user 12% kernel 30%"), "{t}");
        assert!(t.contains("commit 20.0 of 40.0 GB"), "{t}");
        assert!(t.contains("leak.exe (7) 90000"), "{t}");
        assert!(t.contains("a.exe (1) 55%") && t.contains("b.exe (2) 120"), "{t}");
        assert!(t.contains("suggestion: kernel memory is high (4.2 GB): possible handle or driver leak; restarting frees it"), "{t}");
        assert!(!t.contains("compressed"), "null is not printed: {t}");
    }

    #[test]
    fn pool_tags_name_known_owners_show_the_untagged_rest_and_the_own_share() {
        let t = render(&sample());
        assert!(t.contains("kernel pool by tag (partial attribution):"), "{t}");
        assert!(t.contains("FMfn  paged  1541 MB  non-paged     0 MB  file-name cache of the filter manager"), "{t}");
        assert!(
            t.contains(
                "Xyz1  paged     3 MB  non-paged     2 MB
"
            ),
            "{t}"
        );
        assert!(t.contains("no tag (large allocations, special pools): 1300 MB"), "{t}");
        assert!(t.contains("axelstra itself: 240 child processes started, 120.0/min, 35 directories watched"), "{t}");
        assert!(t.contains("biggest pool tag FMfn, file-name cache"), "{t}");
        // an old daemon or Linux: none of it prints
        let mut r = sample();
        r["breakdown"]["pool_tags"] = Value::Null;
        r["breakdown"]["own"] = Value::Null;
        let t = render(&r);
        assert!(!t.contains("by tag") && !t.contains("axelstra itself"), "{t}");
    }

    #[test]
    fn no_suggestion_when_the_kernel_is_not_high_and_nothing_without_a_breakdown() {
        let mut r = sample();
        r["breakdown"]["kernel"]["high"] = json!(false);
        assert!(!render(&r).contains("suggestion"));
        let mut old = sample();
        old.as_object_mut().unwrap().remove("breakdown");
        assert!(!render(&old).contains("where the memory goes"));
    }
}
