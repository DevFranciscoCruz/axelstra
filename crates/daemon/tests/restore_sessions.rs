//! Live sessions of existing worktrees must survive a daemon restart: dispatched leads (spawned with `reap`) are saved
//! too, and `session.pending` hands out every saved session, with no cap.
mod common;
use axelstra_daemon::client::Client;
use axelstra_daemon::server;
use serde_json::{json, Value};
use std::path::Path;
use std::time::Duration;

fn start(root: &Path, tag: &str) -> (Client, String) {
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
    (Client::connect(&pipe, &token).unwrap(), pipe)
}

fn shell() -> (String, Vec<String>) {
    if cfg!(windows) {
        ("cmd".into(), vec!["/q".into()])
    } else {
        ("sh".into(), vec![])
    }
}

fn saved(root: &Path) -> Vec<Value> {
    let j: Value = serde_json::from_str(&std::fs::read_to_string(root.join("sessions.json")).unwrap()).unwrap();
    j["data"].as_array().cloned().unwrap_or_default()
}

fn temp(tag: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!("axelstra-restore-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    root
}

#[test]
fn dispatched_lead_is_saved_for_the_restart() {
    let root = temp("lead");
    let wt = root.join("wt");
    std::fs::create_dir_all(&wt).unwrap();
    let (mut c, _) = start(&root, "lead");
    let (prog, args) = shell();
    let meta = json!({"project": "p", "worktree": wt.to_string_lossy(), "kind": "agent", "agent": "claude", "key": "lead-key-1"});
    let id = c
        .call(
            "session.spawn",
            json!({"program": prog, "args": args, "cwd": wt.to_string_lossy(), "label": "claude", "meta": meta,
                   "reap": {"mode": "exit", "lead": true}}),
        )
        .unwrap()["id"]
        .as_u64()
        .unwrap();
    let rec = saved(&root).into_iter().find(|r| r["meta"]["key"] == "lead-key-1").expect("dispatched lead missing from sessions.json");
    assert!(rec["reap"].is_null(), "the reaper must not be restored");
    assert_eq!(rec["meta"]["lead"], true);
    // a reaped plain shell is not kept
    let (prog, args) = shell();
    c.call("session.spawn", json!({"program": prog, "args": args, "label": "sh", "reap": {"mode": "idle", "idle_secs": 600}})).unwrap();
    assert_eq!(saved(&root).len(), 1);
    let _ = c.call("session.kill", json!({"id": id}));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn pending_hands_out_every_saved_session() {
    let root = temp("many");
    let wt = root.join("wt");
    std::fs::create_dir_all(&wt).unwrap();
    let recs: Vec<Value> = (0..12)
        .map(|i| {
            json!({"program": "cmd", "args": ["/q"], "cwd": wt.to_string_lossy(), "label": format!("a{i}"),
                   "meta": {"project": "p", "worktree": wt.to_string_lossy(), "kind": "agent", "agent": "claude", "key": format!("k{i}")}})
        })
        .collect();
    std::fs::write(root.join("sessions.json"), serde_json::to_string(&json!({"v": 1, "data": recs})).unwrap()).unwrap();
    let (mut c, _) = start(&root, "many");
    std::thread::sleep(Duration::from_millis(100));
    let v = c.call("session.pending", json!({})).unwrap();
    assert_eq!(v.as_array().unwrap().len(), 12, "no cap may drop or delay saved sessions");
    assert_eq!(saved(&root).len(), 12, "nothing leaves the file until it is restored");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn spawn_and_pending_are_idempotent_per_key() {
    let root = temp("idem");
    let wt = root.join("wt");
    std::fs::create_dir_all(&wt).unwrap();
    let meta = json!({"project": "p", "worktree": wt.to_string_lossy(), "kind": "agent", "agent": "claude", "key": "same-key"});
    let (prog, args) = shell();
    let rec = json!({"program": prog, "args": args, "cwd": wt.to_string_lossy(), "label": "a", "meta": meta});
    std::fs::write(root.join("sessions.json"), serde_json::to_string(&json!({"v": 1, "data": [rec.clone()]})).unwrap()).unwrap();
    let (mut c, _) = start(&root, "idem");
    assert_eq!(c.call("session.pending", json!({})).unwrap().as_array().unwrap().len(), 1);
    let first = c.call("session.spawn", rec.clone()).unwrap()["id"].as_u64().unwrap();
    let again = c.call("session.spawn", rec).unwrap();
    assert_eq!(again["id"].as_u64().unwrap(), first, "same key must return the running session");
    let live = c.call("session.list", json!({})).unwrap().as_array().unwrap().len();
    assert_eq!(live, 1);
    assert!(c.call("session.pending", json!({})).unwrap().as_array().unwrap().is_empty(), "a live key is not handed out again");
    let _ = c.call("session.kill", json!({"id": first}));
    let _ = std::fs::remove_dir_all(&root);
}
