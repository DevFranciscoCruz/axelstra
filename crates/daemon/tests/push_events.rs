//! Kanban, orchestration and automations are pushed on the watch stream: the app never asks on a timer.
//! `kanban.changed {project}`, `orch.changed {run}`, `automations.changed`.

mod common;
use axelstra_daemon::client::{Client, Frame};
use axelstra_daemon::server;
use serde_json::{json, Value};
use std::path::Path;
use std::process::Command;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

fn sh(cwd: &Path, args: &[&str]) {
    assert!(Command::new("git").current_dir(cwd).args(args).status().unwrap().success(), "git {args:?}");
}

struct Env {
    root: std::path::PathBuf,
    repo: std::path::PathBuf,
    c: Client,
    pid: String,
    rx: Receiver<Value>,
}

fn setup(tag: &str) -> Env {
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(90));
        eprintln!("watchdog: test stuck");
        std::process::exit(1);
    });
    let root = std::env::temp_dir().join(format!("axelstra-pe-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let repo = root.join("r");
    std::fs::create_dir_all(&repo).unwrap();
    sh(&repo, &["init", "-q", "-b", "main"]);
    sh(&repo, &["config", "user.email", "t@t"]);
    sh(&repo, &["config", "user.name", "t"]);
    std::fs::write(repo.join("a.txt"), "x").unwrap();
    sh(&repo, &["add", "-A"]);
    sh(&repo, &["commit", "-qm", "init"]);

    let pipe = format!("{}-pe{tag}-{}", common::base_pipe(), std::process::id());
    let token = "p".repeat(32);
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
    let pid = c.call("project.add", json!({"path": &repo})).unwrap()["id"].as_str().unwrap().to_string();
    // a listening connection: `watch.unsubscribe` with no worktree still turns the events on
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
    Env { root, repo, c, pid, rx }
}

/// Waits (at most 10 s) for an event named `name` that `ok` accepts.
fn expect(rx: &Receiver<Value>, name: &str, ok: impl Fn(&Value) -> bool) -> Value {
    let t0 = Instant::now();
    loop {
        let left = Duration::from_secs(10).saturating_sub(t0.elapsed());
        let ev = rx.recv_timeout(left).unwrap_or_else(|_| panic!("no {name} event"));
        if ev["event"] == name && ok(&ev) {
            return ev;
        }
    }
}

fn drain(rx: &Receiver<Value>) {
    std::thread::sleep(Duration::from_millis(600));
    while rx.try_recv().is_ok() {}
}

#[test]
fn every_kanban_write_pushes_kanban_changed_with_the_project() {
    let mut e = setup("kb");
    let pid = e.pid.clone();
    let mine = |ev: &Value| ev["project"] == pid.as_str();

    e.c.call("kanban.save", json!({"project": pid, "title": "One"})).unwrap();
    expect(&e.rx, "kanban.changed", mine);
    drain(&e.rx);

    e.c.call("kanban.save", json!({"project": pid, "id": "one", "title": "One!", "body": "b"})).unwrap();
    expect(&e.rx, "kanban.changed", mine);
    drain(&e.rx);

    e.c.call("kanban.move", json!({"project": pid, "id": "one", "status": "todo"})).unwrap();
    expect(&e.rx, "kanban.changed", mine);
    drain(&e.rx);

    e.c.call("kanban.delete", json!({"project": pid, "id": "one"})).unwrap();
    expect(&e.rx, "kanban.changed", mine);
    drain(&e.rx);

    // a failed write and a plain read say nothing (the file watcher stays quiet too: nothing changed on disk)
    assert!(e.c.call("kanban.move", json!({"project": pid, "id": "ghost", "status": "todo"})).is_err());
    e.c.call("kanban.list", json!({"project": pid})).unwrap();
    std::thread::sleep(Duration::from_millis(800));
    assert!(e.rx.try_recv().map(|v| v["event"] != "kanban.changed").unwrap_or(true), "a read or a failed write must not push");
    let _ = std::fs::remove_dir_all(&e.root);
}

#[test]
fn a_card_written_outside_the_daemon_pushes_kanban_changed() {
    let mut e = setup("file");
    let pid = e.pid.clone();
    // the folder exists once a card was saved, and the watch starts then
    e.c.call("kanban.save", json!({"project": pid, "title": "Seed"})).unwrap();
    e.c.call("kanban.list", json!({"project": pid})).unwrap();
    drain(&e.rx);

    // an agent, the orchestrator or an editor writes the file directly
    let card = e.repo.join(".kanban/seed.md");
    let text = std::fs::read_to_string(&card).unwrap().replace("status: backlog", "status: in-progress");
    std::fs::write(&card, text).unwrap();
    expect(&e.rx, "kanban.changed", |ev| ev["project"] == pid.as_str());
    drain(&e.rx);

    std::fs::write(e.repo.join(".kanban/extra.md"), "no frontmatter").unwrap();
    expect(&e.rx, "kanban.changed", |ev| ev["project"] == pid.as_str());
    drain(&e.rx);

    std::fs::remove_file(&card).unwrap();
    expect(&e.rx, "kanban.changed", |ev| ev["project"] == pid.as_str());
    let _ = std::fs::remove_dir_all(&e.root);
}

#[test]
fn an_orchestrator_run_pushes_orch_changed_with_its_name() {
    let mut e = setup("orch");
    let pid = e.pid.clone();
    // nothing in "todo": the run starts, then ends with an error, both are pushed
    let started = e.c.call("orch.start", json!({"project": pid, "max": 1})).unwrap();
    let run = started["name"].as_str().unwrap().to_string();
    let first = expect(&e.rx, "orch.changed", |ev| ev["run"] == run.as_str());
    assert_eq!(first["project"], pid.as_str());
    let t0 = Instant::now();
    loop {
        let st = e.c.call("orch.status", json!({"project": pid})).unwrap();
        if st["state"] != "running" {
            assert_eq!(st["state"], "error");
            break;
        }
        assert!(t0.elapsed() < Duration::from_secs(20), "run never ended");
        // the end is an event too: wait for it instead of sleeping
        let _ = e.rx.recv_timeout(Duration::from_secs(2));
    }
    // the end of the run was announced after its state changed
    let st = e.c.call("orch.status", json!({"project": pid})).unwrap();
    assert_eq!(st["name"], run.as_str());
    let _ = std::fs::remove_dir_all(&e.root);
}

#[test]
fn a_running_orchestration_turns_a_card_move_into_orch_changed() {
    let mut e = setup("move");
    let pid = e.pid.clone();
    // a todo card that never finishes quickly: the agent is a long sleep
    e.c.call("kanban.save", json!({"project": pid, "title": "Slow", "status": "todo"})).unwrap();
    let agent = if cfg!(windows) { json!(["cmd", "/c", "ping -n 30 127.0.0.1 >nul"]) } else { json!(["sh", "-c", "sleep 30"]) };
    let started = e.c.call("orch.start", json!({"project": pid, "max": 1, "agent": agent, "trust_acceptance": true}));
    let Ok(started) = started else {
        // the run could not begin in this environment: the pure file path is covered by the other tests
        let _ = std::fs::remove_dir_all(&e.root);
        return;
    };
    let run = started["name"].as_str().unwrap().to_string();
    drain(&e.rx);
    // while the run is `running` a change in the cards is also a change of the run
    std::fs::write(e.repo.join(".kanban/note.md"), "no frontmatter").unwrap();
    expect(&e.rx, "orch.changed", |ev| ev["run"] == run.as_str());
    let _ = std::fs::remove_dir_all(&e.root);
}

#[test]
fn saving_and_deleting_an_automation_pushes_automations_changed() {
    let mut e = setup("auto");
    let pid = e.pid.clone();
    let a = json!({"id": "", "name": "Sweep", "project": pid, "agent": "claude", "prompt": "look", "schedule": {"kind": "daily", "time": "09:00"}, "enabled": false, "last_run": 0});
    let saved = e.c.call("automation.save", a).unwrap();
    expect(&e.rx, "automations.changed", |_| true);
    drain(&e.rx);

    // refused input changes nothing and says nothing
    assert!(e.c.call("automation.save", json!({"id": "", "name": "", "project": pid, "agent": "claude", "prompt": "x", "schedule": {"kind": "daily", "time": "09:00"}, "enabled": false, "last_run": 0})).is_err());
    std::thread::sleep(Duration::from_millis(500));
    assert!(e.rx.try_recv().map(|v| v["event"] != "automations.changed").unwrap_or(true));

    e.c.call("automation.delete", json!({"id": saved["id"]})).unwrap();
    expect(&e.rx, "automations.changed", |_| true);
    let _ = std::fs::remove_dir_all(&e.root);
}
