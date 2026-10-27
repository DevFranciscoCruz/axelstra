//! `usage.changed` / `limits.changed` come from a file watcher on the agents' usage files, `resources.changed` from the
//! sessions starting and ending; none of them is a timer. The budget loop wakes on the same events.
mod common;
use axelstra_daemon::client::{Client, Frame};
use axelstra_daemon::server;
use serde_json::{json, Value};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

fn start(root: &std::path::Path) -> (Client, Receiver<Value>) {
    std::fs::create_dir_all(root).unwrap();
    let pipe = format!("{}-usage-{}", common::base_pipe(), std::process::id());
    let token = "u".repeat(32);
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
    let c = Client::connect(&pipe, &token).unwrap();
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
    (c, rx)
}

/// True when an event named `name` arrives within `secs`.
fn got(rx: &Receiver<Value>, name: &str, secs: u64) -> bool {
    let end = Instant::now() + Duration::from_secs(secs);
    while let Some(left) = end.checked_duration_since(Instant::now()) {
        match rx.recv_timeout(left) {
            Ok(e) if e["event"] == name => return true,
            Ok(_) => {}
            Err(_) => return false,
        }
    }
    false
}

fn drain(rx: &Receiver<Value>) {
    while rx.try_recv().is_ok() {}
}

#[test]
fn usage_files_and_sessions_push_events() {
    let root = std::env::temp_dir().join(format!("axelstra-usage-ev-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let home = root.join("home");
    std::fs::create_dir_all(home.join(".claude").join("projects")).unwrap();
    std::env::set_var("USERPROFILE", &home);
    std::env::set_var("HOME", &home);
    let data = root.join("d");
    let (mut c, rx) = start(&data);
    // the watcher is armed by the server start; give the OS watch a moment to exist
    std::thread::sleep(Duration::from_millis(500));

    // Claude transcript grows: tokens changed, the rate limits did not
    drain(&rx);
    let dir = home.join(".claude").join("projects").join("x");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("s.jsonl"), "{}\n").unwrap();
    assert!(got(&rx, "usage.changed", 10), "no usage.changed after a transcript write");

    // the status line file of an agent: limits
    drain(&rx);
    std::fs::create_dir_all(data.join("status")).unwrap();
    std::fs::write(data.join("status").join("k.json"), "{}").unwrap();
    assert!(got(&rx, "limits.changed", 10), "no limits.changed after a status file write");

    // `~/.codex` did not exist at start: once it appears its files count for both
    drain(&rx);
    let codex = home.join(".codex").join("sessions");
    std::fs::create_dir_all(&codex).unwrap();
    std::thread::sleep(Duration::from_millis(300));
    std::fs::write(codex.join("r.jsonl"), "{}\n").unwrap();
    let (mut usage, mut limits) = (false, false);
    let end = Instant::now() + Duration::from_secs(10);
    while !(usage && limits) && Instant::now() < end {
        if let Ok(e) = rx.recv_timeout(Duration::from_millis(200)) {
            usage |= e["event"] == "usage.changed";
            limits |= e["event"] == "limits.changed";
        }
    }
    assert!(usage && limits, "codex file: usage {usage}, limits {limits}");

    // a session that starts and ends moves the resources
    drain(&rx);
    let (prog, args): (&str, Vec<&str>) = if cfg!(windows) { ("cmd", vec!["/c", "exit 0"]) } else { ("sh", vec!["-c", "exit 0"]) };
    c.call("session.spawn", json!({"program": prog, "args": args})).unwrap();
    assert!(got(&rx, "resources.changed", 10), "no resources.changed for a session");

    // the budget loop wakes on the usage watcher: a transcript over the limit is reported with no `budget.check` call
    let wt = root.join("wt").to_string_lossy().replace('\\', "/");
    c.call("settings.set", json!({"budgets": {"worktrees": {wt.clone(): {"daily": 1000}}}})).unwrap();
    std::thread::sleep(Duration::from_millis(1200));
    let tdir = home.join(".claude").join("projects").join(axelstra_daemon::agentlog::claude_dir_name(&wt));
    std::fs::create_dir_all(&tdir).unwrap();
    let line = json!({"message": {"id": "m1", "usage": {"input_tokens": 0, "output_tokens": 1500}}});
    std::fs::write(
        tdir.join("s.jsonl"),
        format!(
            "{line}
"
        ),
    )
    .unwrap();
    let end = Instant::now() + Duration::from_secs(15);
    let mut over = false;
    while !over && Instant::now() < end {
        let ev = c.call("budget.events", json!({})).unwrap();
        over = ev["events"].as_array().unwrap().iter().any(|e| e["level"] == "over");
        if !over {
            std::thread::sleep(Duration::from_millis(200));
        }
    }
    assert!(over, "the budget loop did not wake on the usage write");

    // nothing happens: nothing is sent (no timer announces)
    std::thread::sleep(Duration::from_millis(1500));
    drain(&rx);
    assert!(!got(&rx, "usage.changed", 2) && !got(&rx, "limits.changed", 0));
    let _ = std::fs::remove_dir_all(&root);
}
