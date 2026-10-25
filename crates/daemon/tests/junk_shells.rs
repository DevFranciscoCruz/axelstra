//! Bare shells (no cwd, no meta) are never saved or restored; a closed session leaves `sessions.json` and `ls` at once;
//! a start prunes the legacy junk after copying the file to `backups/`.
mod common;
use axelstra_daemon::client::{Client, Frame};
use axelstra_daemon::server;
use serde_json::{json, Value};
use std::path::Path;
use std::sync::mpsc;
use std::time::{Duration, Instant};

fn start(root: &Path, tag: &str) -> Client {
    let pipe = format!("{}-{tag}-{}", common::base_pipe(), std::process::id());
    let token = "c".repeat(32);
    let (p, t, s) = (pipe.clone(), token.clone(), root.join("projects.json"));
    std::thread::spawn(move || {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap()
            .block_on(server::serve_with_store(p, t, s))
            .unwrap();
    });
    common::wait_ready(&pipe, &token);
    Client::connect(&pipe, &token).unwrap()
}

fn shell() -> (&'static str, Vec<&'static str>) {
    if cfg!(windows) {
        ("cmd", vec!["/q"])
    } else {
        ("sh", vec![])
    }
}

fn saved(root: &Path) -> Vec<Value> {
    let j: Value = match std::fs::read_to_string(root.join("sessions.json")) {
        Ok(t) => serde_json::from_str(&t).unwrap(),
        Err(_) => return vec![],
    };
    j["data"].as_array().cloned().unwrap_or_default()
}

fn temp(tag: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!("axelstra-junk-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    root
}

fn until(mut f: impl FnMut() -> bool) -> bool {
    let end = Instant::now() + Duration::from_secs(10);
    while Instant::now() < end {
        if f() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

#[test]
fn a_bare_shell_is_never_saved_and_a_killed_session_leaves_file_and_list() {
    let root = temp("kill");
    let wt = root.join("wt");
    std::fs::create_dir_all(&wt).unwrap();
    let mut c = start(&root, "kill");
    let (prog, args) = shell();
    let bare = c.call("session.spawn", json!({"program": prog, "args": args})).unwrap()["id"].as_u64().unwrap();
    let kept = c
        .call(
            "session.spawn",
            json!({"program": prog, "args": args, "cwd": wt.to_string_lossy(), "label": "t",
                   "meta": {"project": "p", "worktree": wt.to_string_lossy(), "kind": "terminal"}}),
        )
        .unwrap()["id"]
        .as_u64()
        .unwrap();
    assert_eq!(saved(&root).len(), 1, "only the terminal with a worktree is saved");
    c.call("session.kill", json!({"id": kept})).unwrap();
    assert!(saved(&root).is_empty(), "a killed session's spec must leave sessions.json");
    let mut ids =
        || -> Vec<u64> { c.call("session.list", json!({})).unwrap().as_array().unwrap().iter().filter_map(|s| s["id"].as_u64()).collect() };
    assert!(!ids().contains(&kept), "a killed session must leave the list");
    let _ = c.call("session.kill", json!({"id": bare}));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_session_that_exits_by_itself_leaves_sessions_json() {
    let root = temp("exit");
    let wt = root.join("wt");
    std::fs::create_dir_all(&wt).unwrap();
    let mut c = start(&root, "exit");
    let (prog, args): (&str, Value) = if cfg!(windows) { ("cmd", json!(["/c", "exit"])) } else { ("sh", json!(["-c", "exit 0"])) };
    c.call("session.spawn", json!({"program": prog, "args": args, "cwd": wt.to_string_lossy(), "meta": {"kind": "terminal"}})).unwrap();
    assert!(until(|| saved(&root).is_empty()), "an exited session must leave sessions.json");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_session_that_exits_by_itself_leaves_the_list_announced_with_its_code() {
    let root = temp("announce");
    let wt = root.join("wt");
    std::fs::create_dir_all(&wt).unwrap();
    let pipe = format!("{}-announce-{}", common::base_pipe(), std::process::id());
    let token = "c".repeat(32);
    let (p, t, s) = (pipe.clone(), token.clone(), root.join("projects.json"));
    std::thread::spawn(move || {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap()
            .block_on(server::serve_with_store(p, t, s))
            .unwrap();
    });
    common::wait_ready(&pipe, &token);
    let mut c = Client::connect(&pipe, &token).unwrap();
    // a listening connection: `watch.unsubscribe` with no worktree still turns on the session events
    let mut w = Client::connect(&pipe, &token).unwrap();
    w.call("watch.unsubscribe", json!({})).unwrap();
    let (tx, rx) = mpsc::channel::<Value>();
    std::thread::spawn(move || {
        while let Ok(f) = w.read() {
            if let Frame::Event(e) = f {
                let _ = tx.send(e);
            }
        }
    });
    let (prog, args): (&str, Value) = if cfg!(windows) { ("cmd", json!(["/c", "exit 3"])) } else { ("sh", json!(["-c", "exit 3"])) };
    let id = c
        .call("session.spawn", json!({"program": prog, "args": args, "cwd": wt.to_string_lossy(), "meta": {"kind": "terminal"}}))
        .unwrap()["id"]
        .as_u64()
        .unwrap();
    let end = Instant::now() + Duration::from_secs(10);
    let ev = loop {
        let left = end.saturating_duration_since(Instant::now());
        let e = rx.recv_timeout(left).expect("no session.exited after the exit");
        if e["event"] == "session.exited" {
            break e;
        }
    };
    assert_eq!((ev["id"].as_u64(), ev["code"].as_i64()), (Some(id), Some(3)));
    let ids: Vec<u64> = c.call("session.list", json!({})).unwrap().as_array().unwrap().iter().filter_map(|s| s["id"].as_u64()).collect();
    assert!(!ids.contains(&id), "an exited session must leave the list");
    assert!(saved(&root).is_empty());
    // the code stays answerable for `axelstra wait` and flows
    let st = c.call("session.status", json!({"id": id})).unwrap();
    assert_eq!((st["running"].as_bool(), st["code"].as_i64(), st["gone"].as_bool()), (Some(false), Some(3), Some(true)));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn start_prunes_legacy_bare_shells_after_a_backup_and_hands_out_none() {
    let root = temp("prune");
    let wt = root.join("wt");
    std::fs::create_dir_all(&wt).unwrap();
    let mut recs: Vec<Value> = (0..20).map(|_| json!({"program": "cmd", "args": ["/q"]})).collect();
    recs.push(json!({"program": "powershell"}));
    recs.push(json!({"program": "cmd", "args": ["/q"], "cwd": wt.to_string_lossy(), "label": "real",
                     "meta": {"project": "p", "worktree": wt.to_string_lossy(), "kind": "terminal"}}));
    std::fs::write(root.join("sessions.json"), serde_json::to_string(&json!({"v": 1, "data": recs})).unwrap()).unwrap();
    let mut c = start(&root, "prune");
    let pending = c.call("session.pending", json!({})).unwrap();
    assert_eq!(pending.as_array().unwrap().len(), 1, "only the real terminal is handed out");
    assert_eq!(saved(&root).len(), 1);
    let backups: Vec<_> = std::fs::read_dir(root.join("backups")).unwrap().filter_map(Result::ok).collect();
    assert!(!backups.is_empty(), "the file as it was is copied to backups/ first");
    let old = std::fs::read_to_string(backups[0].path()).unwrap();
    assert_eq!(serde_json::from_str::<Value>(&old).unwrap()["data"].as_array().unwrap().len(), 22);
    let _ = std::fs::remove_dir_all(&root);
}
