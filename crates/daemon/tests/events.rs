//! The daemon reacts to events, not timers: a session's exit, a worktree that disappeared.

use axelstra_daemon::client::{Client, Frame};
use axelstra_daemon::server;
use serde_json::{json, Value};
use std::sync::mpsc;
use std::time::{Duration, Instant};

fn start(tag: &str) -> (std::path::PathBuf, String, String) {
    let root = std::env::temp_dir().join(format!("axelstra-ev-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let pipe = format!("{}-ev-{tag}-{}", axelstra_daemon::proto::pipe_name(), std::process::id());
    let token = "e".repeat(32);
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
    std::thread::sleep(Duration::from_millis(300));
    (root, pipe, token)
}

fn watchdog(secs: u64) {
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(secs));
        eprintln!("watchdog: test stuck");
        std::process::exit(1);
    });
}

fn running(c: &mut Client, id: u64) -> bool {
    let list = c.call("session.list", json!({})).unwrap();
    list.as_array().unwrap().iter().any(|s| s["id"].as_u64() == Some(id) && s["running"] == true)
}

/// A shell that leaves a background child behind and exits; the child would write `marker` a few seconds later.
fn leaves_a_child(dir: &std::path::Path) -> (String, Vec<String>) {
    if cfg!(windows) {
        let script = dir.join("leave.cmd");
        std::fs::write(&script, "@start \"\" /b cmd /c \"ping -n 4 127.0.0.1 >nul & echo x > \"%~dp0marker\"\"\r\n@exit 0\r\n").unwrap();
        ("cmd".into(), vec!["/c".into(), script.to_string_lossy().into_owned()])
    } else {
        let line = format!("(sleep 3; touch '{}') & exit 0", dir.join("marker").display());
        ("sh".into(), vec!["-c".into(), line])
    }
}

#[test]
fn exit_is_pushed_and_ends_what_the_session_left_running() {
    watchdog(60);
    let (root, pipe, token) = start("exit");
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

    let (program, args) = leaves_a_child(&root);
    let id = c.call("session.spawn", json!({"program": program, "args": args, "cwd": root})).unwrap()["id"].as_u64().unwrap();
    // the exit arrives as an event: wait for the one after which the session no longer runs
    let t0 = Instant::now();
    loop {
        let left = Duration::from_secs(15).saturating_sub(t0.elapsed());
        let ev = rx.recv_timeout(left).expect("no session.changed after the exit");
        // `session.exited` and `dispatch.changed` come with the exit too
        if ev["event"] != "session.changed" {
            continue;
        }
        if !running(&mut c, id) {
            break;
        }
    }
    // the background child went with it: its marker never appears
    std::thread::sleep(Duration::from_secs(6));
    assert!(!root.join("marker").exists(), "a child of an exited session kept running");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_vanished_worktree_folder_ends_its_sessions() {
    watchdog(60);
    let (root, pipe, token) = start("orphan");
    let mut c = Client::connect(&pipe, &token).unwrap();
    let wt = root.join("wt-gone");
    std::fs::create_dir_all(&wt).unwrap();
    let (program, args): (&str, Vec<&str>) = if cfg!(windows) { ("cmd", vec!["/q"]) } else { ("sh", vec![]) };
    // the shell runs elsewhere (a cwd inside would keep the folder from being deleted on Windows)
    let meta = json!({"worktree": wt, "kind": "terminal"});
    let id = c.call("session.spawn", json!({"program": program, "args": args, "cwd": root, "meta": meta})).unwrap()["id"].as_u64().unwrap();
    let other = c.call("session.spawn", json!({"program": program, "args": args, "cwd": root})).unwrap()["id"].as_u64().unwrap();
    assert!(running(&mut c, id));

    std::fs::remove_dir_all(&wt).unwrap();
    // asking for the project list is one of the moments the daemon notices it
    c.call("project.list", json!({})).unwrap();
    let t0 = Instant::now();
    while running(&mut c, id) {
        assert!(t0.elapsed() < Duration::from_secs(10), "session of a deleted worktree still runs");
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(running(&mut c, other), "a session without a worktree is left alone");
    let _ = std::fs::remove_dir_all(&root);
}
