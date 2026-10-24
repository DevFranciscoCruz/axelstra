mod common;
use axelstra_daemon::client::Client;
use axelstra_daemon::server;
use serde_json::json;
use std::time::{Duration, Instant};

fn shell() -> (String, Vec<String>) {
    if cfg!(windows) {
        ("cmd".into(), vec!["/q".into()])
    } else {
        ("sh".into(), vec![])
    }
}

fn ids(c: &mut Client) -> Vec<u64> {
    c.call("session.list", json!({})).unwrap().as_array().unwrap().iter().map(|s| s["id"].as_u64().unwrap()).collect()
}

#[test]
fn finished_worker_reports_to_parent_and_disappears() {
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(60));
        eprintln!("watchdog: test stuck");
        std::process::exit(1);
    });
    let root = std::env::temp_dir().join(format!("axelstra-reap-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let pipe = format!("{}-reap-{}", common::base_pipe(), std::process::id());
    let token = "b".repeat(32);
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
    let mut c = Client::connect(&pipe, &token).unwrap();

    let (prog, args) = shell();
    let parent = c.call("session.spawn", json!({"program": prog, "args": args, "label": "main", "meta": {"kind": "agent"}})).unwrap()["id"]
        .as_u64()
        .unwrap();
    let (prog, mut args) = shell();
    args.extend(if cfg!(windows) {
        vec!["/c".to_string(), "echo tests-passed-42".to_string()]
    } else {
        vec!["-c".to_string(), "echo tests-passed-42".to_string()]
    });
    let worker = c
        .call("session.spawn", json!({"program": prog, "args": args, "label": "tests", "reap": {"parent": parent, "lines": 10}}))
        .unwrap()["id"]
        .as_u64()
        .unwrap();
    assert!(ids(&mut c).contains(&worker));

    // the worker closes itself once it ended
    let t0 = Instant::now();
    while ids(&mut c).contains(&worker) {
        assert!(t0.elapsed() < Duration::from_secs(15), "worker never reaped");
        std::thread::sleep(Duration::from_millis(200));
    }
    assert!(ids(&mut c).contains(&parent), "parent stays");

    // and the parent's screen shows what the worker printed
    c.call("session.subscribe", json!({"id": parent})).unwrap();
    let t0 = Instant::now();
    let mut seen = String::new();
    while !seen.contains("tests-passed-42") {
        assert!(t0.elapsed() < Duration::from_secs(10), "parent never got the report: {seen}");
        if let Ok(axelstra_daemon::client::Frame::Output { session, data }) = c.read() {
            if session as u64 == parent {
                seen.push_str(&String::from_utf8_lossy(&data));
            }
        }
    }
    let _ = std::fs::remove_dir_all(&root);
}
