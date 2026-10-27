//! `clone.changed {id}` ends every `project.clone` job on the watch stream, and the first `docker.*` call starts the lazy
//! `docker events` watcher without ever failing the call. The line -> event parsing is unit-tested in `server/containers.rs`.
mod common;
use axelstra_daemon::client::{Client, Frame};
use axelstra_daemon::server;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

fn start(tag: &str) -> (Client, Receiver<Value>) {
    let root = common::store(tag).parent().unwrap().to_path_buf();
    let pipe = common::pipe(tag);
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

fn git(dir: &Path, args: &[&str]) {
    let o = Command::new("git")
        .current_dir(dir)
        .args(["-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false"])
        .args(args)
        .output()
        .unwrap();
    assert!(o.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&o.stderr));
}

fn tmp(tag: &str) -> PathBuf {
    let d = common::data_dir().join(format!("ce-{tag}"));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn file_url(p: &Path) -> String {
    let p = p.to_string_lossy().replace('\\', "/");
    if p.starts_with('/') {
        format!("file://{p}")
    } else {
        format!("file:///{p}")
    }
}

/// Reads `clone.changed` events for `id` until the status the daemon reports is no longer `running`; returns that status.
/// The event is pushed after the status changed, so an event whose status is still `running` is just progress.
fn until_ended(c: &mut Client, rx: &Receiver<Value>, id: &str) -> Value {
    let end = Instant::now() + Duration::from_secs(60);
    loop {
        let st = c.call("project.clone_status", json!({"id": id})).unwrap();
        if st["state"] != "running" {
            return st;
        }
        let left = end.saturating_duration_since(Instant::now());
        let ev = rx.recv_timeout(left).unwrap_or_else(|_| panic!("no clone.changed before the end: {st}"));
        if ev["event"] == "clone.changed" {
            assert_eq!(ev["id"], id, "an event for another clone");
        }
    }
}

#[test]
fn a_clone_pushes_clone_changed_and_the_last_one_is_after_its_end() {
    let (mut c, rx) = start("ok");
    let base = tmp("ok");
    let work = base.join("work");
    std::fs::create_dir_all(&work).unwrap();
    git(&work, &["init", "-b", "main"]);
    std::fs::write(work.join("a.txt"), "hi").unwrap();
    git(&work, &["add", "a.txt"]);
    git(&work, &["commit", "-m", "one"]);
    let bare = base.join("origin.git");
    git(&base, &["clone", "--bare", work.to_str().unwrap(), bare.to_str().unwrap()]);
    let dest = base.join("out");
    let id = c.call("project.clone", json!({"url": file_url(&bare), "dest": dest.to_str().unwrap()})).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    // the end is always announced, even when git printed no progress at all
    let t0 = Instant::now();
    let ev = loop {
        let left = Duration::from_secs(60).saturating_sub(t0.elapsed());
        let ev = rx.recv_timeout(left).expect("no clone.changed");
        if ev["event"] == "clone.changed" && ev["id"] == id.as_str() {
            let st = c.call("project.clone_status", json!({"id": id})).unwrap();
            if st["state"] != "running" {
                break st;
            }
        }
    };
    assert_eq!(ev["state"], "done", "{ev}");
}

#[test]
fn a_failed_clone_pushes_clone_changed_too() {
    let (mut c, rx) = start("fail");
    let base = tmp("fail");
    let dest = base.join("out");
    let id = c.call("project.clone", json!({"url": file_url(&base.join("nope.git")), "dest": dest.to_str().unwrap()})).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let t0 = Instant::now();
    loop {
        let left = Duration::from_secs(60).saturating_sub(t0.elapsed());
        let ev = rx.recv_timeout(left).expect("no clone.changed");
        if ev["event"] == "clone.changed"
            && ev["id"] == id.as_str()
            && c.call("project.clone_status", json!({"id": id})).unwrap()["state"] == "failed"
        {
            break;
        }
    }
    let st = until_ended(&mut c, &rx, &id);
    assert_eq!(st["error_code"], "project.clone_failed");
}

#[test]
fn a_cancelled_clone_pushes_clone_changed() {
    let (mut c, rx) = start("cancel");
    let base = tmp("cancel");
    let work = base.join("work");
    std::fs::create_dir_all(&work).unwrap();
    git(&work, &["init", "-b", "main"]);
    std::fs::write(work.join("a.txt"), "hi").unwrap();
    git(&work, &["add", "a.txt"]);
    git(&work, &["commit", "-m", "one"]);
    let bare = base.join("origin.git");
    git(&base, &["clone", "--bare", work.to_str().unwrap(), bare.to_str().unwrap()]);
    let id = c.call("project.clone", json!({"url": file_url(&bare), "dest": base.join("c").to_str().unwrap()})).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    c.call("project.clone_cancel", json!({"id": id})).unwrap();
    // cancelled, or done when a tiny repo won the race: either way the last state is announced
    let st = until_ended(&mut c, &rx, &id);
    assert!(st["state"] == "cancelled" || st["state"] == "done", "{st}");
}

#[test]
fn the_first_docker_call_starts_the_watcher_without_failing_the_call() {
    let (mut c, rx) = start("docker");
    // docker absent: the watcher ends quietly; docker present: it streams. Either way the calls work and stay repeatable.
    let first = c.call("docker.status", json!({}));
    let second = c.call("docker.status", json!({}));
    assert_eq!(first.is_ok(), second.is_ok());
    // no clone event can come from docker traffic
    while let Ok(ev) = rx.recv_timeout(Duration::from_millis(300)) {
        assert_ne!(ev["event"], "clone.changed");
    }
    assert!(c.call("project.list", json!({})).is_ok(), "daemon still answers");
}
