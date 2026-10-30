//! `axelstra disk` and the report of `axelstra worktree sweep --artifacts`: disk use of the worktrees from the terminal.

use anyhow::Result;
use axelstra_daemon::client::Client;
use serde_json::{json, Value};
use std::fmt::Write;

pub fn status(mut c: Client, as_json: bool, refresh: bool) -> Result<()> {
    let r = c.call("disk.status", json!({"refresh": refresh}))?;
    if as_json {
        println!("{}", serde_json::to_string_pretty(&r)?);
        return Ok(());
    }
    print!("{}", render(&r, now_ms()));
    Ok(())
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// A size given in MB: GB with one decimal from 1 GB up, else MB.
pub(crate) fn size(mb: u64) -> String {
    if mb >= 1024 {
        format!("{:.1} GB", mb as f64 / 1024.0)
    } else {
        format!("{mb} MB")
    }
}

/// How long ago `then_ms` was, short ("5 min ago", "3 h ago", "2 d ago"); 0 or absent is "unknown".
fn ago(then_ms: u64, now_ms: u64) -> String {
    if then_ms == 0 {
        return "unknown".into();
    }
    let s = now_ms.saturating_sub(then_ms) / 1000;
    match s {
        0..=59 => "just now".into(),
        60..=3599 => format!("{} min ago", s / 60),
        3600..=86_399 => format!("{} h ago", s / 3600),
        _ => format!("{} d ago", s / 86_400),
    }
}

fn n(v: &Value) -> u64 {
    v.as_u64().unwrap_or(0)
}

fn render(r: &Value, now: u64) -> String {
    let mut o = String::new();
    let vols = r["volumes"].as_array().cloned().unwrap_or_default();
    if !vols.is_empty() {
        let _ = writeln!(o, "volumes:");
        for v in &vols {
            let low =
                if v["low"].as_bool() == Some(true) { format!("  LOW (below {})", size(n(&v["min_free_mb"]))) } else { String::new() };
            let _ = writeln!(
                o,
                "  {:<24} {} free of {}{low}",
                v["root"].as_str().unwrap_or("?"),
                size(n(&v["free_mb"])),
                size(n(&v["total_mb"]))
            );
        }
    }
    let mut wts = r["worktrees"].as_array().cloned().unwrap_or_default();
    if wts.is_empty() {
        let _ = writeln!(o, "no worktrees");
        return o;
    }
    wts.sort_by_key(|w| std::cmp::Reverse(n(&w["total_mb"])));
    let rows: Vec<[String; 9]> = wts
        .iter()
        .map(|w| {
            let state = if w["integrated"].as_bool() == Some(true) {
                "integrated".to_string()
            } else {
                match n(&w["ahead"]) {
                    0 => "-".to_string(),
                    a => format!("{a} ahead"),
                }
            };
            let sessions = w["sessions"].as_array().map(|s| s.len()).unwrap_or(0);
            let sessions = if sessions == 0 { "-".to_string() } else { sessions.to_string() };
            let branch = w["branch"].as_str().filter(|b| !b.is_empty()).unwrap_or_else(|| w["path"].as_str().unwrap_or("?"));
            [
                branch.to_string(),
                size(n(&w["total_mb"])),
                size(n(&w["target_mb"])),
                size(n(&w["node_modules_mb"])),
                size(n(&w["cache_mb"])),
                ago(n(&w["last_used_ms"]), now),
                state,
                sessions,
                size(n(&w["reclaimable_mb"])),
            ]
        })
        .collect();
    let head = ["branch", "total", "target", "node_modules", "cache", "last used", "state", "sessions", "reclaimable"];
    let mut width = head.map(str::len);
    for r in &rows {
        for (i, c) in r.iter().enumerate() {
            width[i] = width[i].max(c.len());
        }
    }
    let line = |o: &mut String, cells: &[&str]| {
        let mut s = String::new();
        for (i, c) in cells.iter().enumerate() {
            // the branch is left aligned, the rest are numbers or short words, right aligned
            if i == 0 {
                let _ = write!(s, "{c:<w$}", w = width[i]);
            } else {
                let _ = write!(s, "  {c:>w$}", w = width[i]);
            }
        }
        let _ = writeln!(o, "{}", s.trim_end());
    };
    line(&mut o, &head);
    for r in &rows {
        line(&mut o, &r.iter().map(String::as_str).collect::<Vec<_>>());
    }
    let total: u64 = wts.iter().map(|w| n(&w["reclaimable_mb"])).sum();
    let _ = writeln!(o, "reclaimable now: {} (`axelstra worktree sweep --artifacts`)", size(total));
    o
}

/// English text of a skip code from `worktree.sweep {artifacts}` (a code we do not know prints as is, with its params).
fn skip_text(code: &str, p: &Value) -> String {
    let s = |k: &str| match &p[k] {
        Value::String(v) => v.clone(),
        Value::Null => "?".into(),
        v => v.to_string(),
    };
    match code {
        "artifacts.busy" => "a session or build is using it".into(),
        "artifacts.recent" => format!("used in the last {} h (--force skips this rule)", s("hours")),
        "artifacts.main" => "the project's main checkout is never cleaned".into(),
        "artifacts.failed" => format!("could not remove: {}", s("detail")),
        _ if p.is_object() && !p.as_object().is_some_and(|m| m.is_empty()) => format!("{code} {p}"),
        _ => code.to_string(),
    }
}

/// The answer of `worktree.sweep {artifacts: true}`: freed size per worktree and per directory, then what was skipped.
pub(crate) fn render_sweep(r: &Value, dry_run: bool) -> String {
    let mut o = String::new();
    let verb = if dry_run { "would free" } else { "freed" };
    let list = r["artifacts"].as_array().cloned().unwrap_or_default();
    for a in &list {
        let _ = writeln!(o, "{verb} {} in {}", size(n(&a["freed_mb"])), a["path"].as_str().unwrap_or("?"));
        for d in a["dirs"].as_array().into_iter().flatten() {
            let _ =
                writeln!(o, "  {:<12} {:>9}  {}", d["kind"].as_str().unwrap_or("?"), size(n(&d["mb"])), d["path"].as_str().unwrap_or("?"));
        }
    }
    let skipped = r["skipped"].as_array().cloned().unwrap_or_default();
    if !skipped.is_empty() {
        let _ = writeln!(o, "skipped:");
        for s in &skipped {
            // an English `error` from the daemon wins over our own wording of the code
            let why = s["error"].as_str().map(str::to_string).unwrap_or_else(|| skip_text(s["code"].as_str().unwrap_or("?"), &s["params"]));
            let _ = writeln!(o, "  {}: {why}", s["path"].as_str().unwrap_or("?"));
        }
    }
    if list.is_empty() && skipped.is_empty() {
        let _ = writeln!(o, "nothing to reclaim");
    } else {
        let _ = writeln!(o, "total {verb}: {}", size(n(&r["freed_mb"])));
    }
    o
}

/// The `disk.ensure` answer as one warning line, only when the disk is still low after the reclaim.
pub(crate) fn ensure_warning(r: &Value) -> Option<String> {
    if r["low"].as_bool() != Some(true) {
        return None;
    }
    let freed = match n(&r["reclaimed_mb"]) {
        0 => String::new(),
        mb => format!(" after reclaiming {}", size(mb)),
    };
    Some(format!("axelstra lock: warning: low disk space: {:.1} GB free{freed} (see `axelstra disk`)", n(&r["free_mb"]) as f64 / 1024.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_800_000_000_000;

    fn sample() -> Value {
        json!({
            "volumes": [{"root": "C:\\", "free_mb": 20480, "total_mb": 953_000, "low": true, "min_free_mb": 30720}],
            "worktrees": [
                {"project": "p", "path": "C:/x/.axelstra-wt/a", "branch": "ax/small", "total_mb": 900, "target_mb": 0,
                 "node_modules_mb": 800, "cache_mb": 0, "last_used_ms": NOW - 5 * 60_000, "integrated": true, "ahead": 0,
                 "sessions": [3], "active": true, "reclaimable_mb": 0},
                {"project": "p", "path": "C:/x/.axelstra-wt/b", "branch": "ax/big", "total_mb": 9318, "target_mb": 8800,
                 "node_modules_mb": 400, "cache_mb": 12, "last_used_ms": NOW - 2 * 86_400_000, "integrated": false, "ahead": 3,
                 "sessions": [], "active": false, "reclaimable_mb": 9212},
            ],
            "measured_ms": NOW,
        })
    }

    #[test]
    fn sizes_and_relative_times() {
        assert_eq!(size(512), "512 MB");
        assert_eq!(size(9318), "9.1 GB");
        assert_eq!(ago(0, NOW), "unknown");
        assert_eq!(ago(NOW - 10_000, NOW), "just now");
        assert_eq!(ago(NOW - 5 * 60_000, NOW), "5 min ago");
        assert_eq!(ago(NOW - 3 * 3_600_000, NOW), "3 h ago");
        assert_eq!(ago(NOW - 2 * 86_400_000, NOW), "2 d ago");
        assert_eq!(ago(NOW + 1000, NOW), "just now");
    }

    #[test]
    fn the_table_lists_the_biggest_first_with_low_volumes_marked() {
        let t = render(&sample(), NOW);
        assert!(t.contains("20.0 GB free of 930.7 GB  LOW (below 30.0 GB)"), "{t}");
        let big = t.find("ax/big").unwrap();
        let small = t.find("ax/small").unwrap();
        assert!(big < small, "biggest first: {t}");
        let row = t.lines().find(|l| l.starts_with("ax/big")).unwrap();
        for cell in ["9.1 GB", "8.6 GB", "400 MB", "2 d ago", "3 ahead", "-"] {
            assert!(row.contains(cell), "{cell} in {row}");
        }
        let row = t.lines().find(|l| l.starts_with("ax/small")).unwrap();
        assert!(row.contains("integrated") && row.contains("5 min ago") && row.ends_with("0 MB"), "{row}");
        assert!(t.contains("reclaimable now: 9.0 GB"), "{t}");
        // the header and the rows line up
        let head = t.lines().find(|l| l.starts_with("branch")).unwrap();
        assert_eq!(head.find("total").map(|i| i + 5), row.find("900 MB").map(|i| i + 6), "{t}");
    }

    #[test]
    fn an_empty_or_partial_answer_still_prints() {
        assert!(render(&json!({}), NOW).contains("no worktrees"));
        let t = render(&json!({"worktrees": [{"path": "C:/w", "total_mb": 5}]}), NOW);
        assert!(t.lines().any(|l| l.starts_with("C:/w") && l.contains("unknown")), "{t}");
    }

    #[test]
    fn the_sweep_report_names_freed_dirs_and_skips_in_english() {
        let r = json!({
            "artifacts": [{"path": "C:/x/.axelstra-wt/b", "freed_mb": 9200, "dirs": [
                {"path": "C:/x/.axelstra-wt/b/target", "kind": "target", "mb": 8800},
                {"path": "C:/x/.axelstra-wt/b/app/node_modules", "kind": "node_modules", "mb": 400}]}],
            "skipped": [
                {"path": "C:/x/.axelstra-wt/a", "code": "artifacts.busy", "params": {"path": "C:/x/.axelstra-wt/a"}},
                {"path": "C:/x/.axelstra-wt/c", "code": "artifacts.recent", "params": {"path": "c", "hours": 6}},
                {"path": "C:/x/.axelstra-wt/d", "code": "artifacts.failed", "params": {"path": "d", "detail": "access denied"}},
                {"path": "C:/x/.axelstra-wt/e", "code": "artifacts.new_thing", "params": {"n": 1}},
                {"path": "C:/x/.axelstra-wt/f", "code": "artifacts.busy", "params": {}, "error": "busy: session 4"}],
            "freed_mb": 9200,
        });
        let t = render_sweep(&r, false);
        assert!(t.contains("freed 9.0 GB in C:/x/.axelstra-wt/b"), "{t}");
        assert!(t.contains("  target          8.6 GB  C:/x/.axelstra-wt/b/target"), "{t}");
        assert!(t.contains("  node_modules    400 MB"), "{t}");
        assert!(t.contains("C:/x/.axelstra-wt/a: a session or build is using it"), "{t}");
        assert!(t.contains("used in the last 6 h (--force skips this rule)"), "{t}");
        assert!(t.contains("could not remove: access denied"), "{t}");
        assert!(t.contains("artifacts.new_thing {\"n\":1}"), "{t}");
        assert!(t.contains("C:/x/.axelstra-wt/f: busy: session 4"), "{t}");
        assert!(t.contains("total freed: 9.0 GB"), "{t}");
        assert!(render_sweep(&r, true).contains("would free 9.0 GB in"));
        assert_eq!(render_sweep(&json!({"artifacts": [], "skipped": [], "freed_mb": 0}), true), "nothing to reclaim\n");
    }

    #[test]
    fn ensure_warns_only_while_still_low() {
        assert_eq!(ensure_warning(&json!({"low": false, "free_mb": 100, "reclaimed_mb": 0})), None);
        assert_eq!(ensure_warning(&json!({})), None);
        assert_eq!(
            ensure_warning(&json!({"low": true, "free_mb": 12_800, "reclaimed_mb": 0})).unwrap(),
            "axelstra lock: warning: low disk space: 12.5 GB free (see `axelstra disk`)"
        );
        assert!(ensure_warning(&json!({"low": true, "free_mb": 1024, "reclaimed_mb": 2048}))
            .unwrap()
            .contains("1.0 GB free after reclaiming 2.0 GB"));
    }

    #[test]
    fn the_flags_parse_and_artifact_flags_need_artifacts() {
        use crate::{Cli, Cmd, WtCmd};
        use clap::Parser;
        let ok = Cli::try_parse_from(["axelstra", "worktree", "sweep", "--artifacts", "--dry-run", "--path", "w", "--force"]).unwrap();
        assert!(matches!(ok.cmd, Cmd::Worktree { cmd: WtCmd::Sweep { artifacts: true, dry_run: true, force: true, path: Some(_), .. } }));
        assert!(Cli::try_parse_from(["axelstra", "worktree", "sweep", "--force"]).is_err());
        assert!(Cli::try_parse_from(["axelstra", "worktree", "sweep", "--path", "w"]).is_err());
        assert!(Cli::try_parse_from(["axelstra", "worktree", "sweep", "--artifacts", "--auto", "on"]).is_err());
        assert!(matches!(
            Cli::try_parse_from(["axelstra", "disk", "--json", "--refresh"]).unwrap().cmd,
            Cmd::Disk { json: true, refresh: true }
        ));
    }
}
