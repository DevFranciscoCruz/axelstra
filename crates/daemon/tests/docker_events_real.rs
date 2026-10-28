//! The lazy `docker events` child against a real docker engine: creating a volume must reach the watch stream as
//! `docker.changed`. Skipped (passes) when no docker engine answers, so CI machines without docker stay green.
mod common;
use axelstra_daemon::client::{Client, Frame};
use axelstra_daemon::server;
use serde_json::{json, Value};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

fn docker_up() -> bool {
    Command::new("docker")
        .args(["info", "--format", "{{.ServerVersion}}"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

#[test]
fn a_real_docker_change_reaches_the_watch_stream() {
    if !docker_up() {
        eprintln!("skipped: no docker engine");
        return;
    }
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(120));
        eprintln!("watchdog: test stuck");
        std::process::exit(1);
    });
    let root = common::store("dockerreal").parent().unwrap().to_path_buf();
    let pipe = common::pipe("dockerreal");
    let token = "d".repeat(32);
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
    // the first docker.* call starts the watcher; give the child a moment to attach, then drop what its attach announced
    c.call("docker.status", json!({})).unwrap();
    std::thread::sleep(Duration::from_secs(3));
    while rx.try_recv().is_ok() {}

    let name = format!("axl-evt-{}", std::process::id());
    assert!(Command::new("docker").args(["volume", "create", &name]).stdout(Stdio::null()).status().unwrap().success());
    let got = loop {
        match rx.recv_timeout(Duration::from_secs(15)) {
            Ok(e) if e["event"] == "docker.changed" => break true,
            Ok(_) => continue,
            Err(_) => break false,
        }
    };
    let _ = Command::new("docker").args(["volume", "rm", &name]).stdout(Stdio::null()).status();
    assert!(got, "docker volume create produced no docker.changed");
}
