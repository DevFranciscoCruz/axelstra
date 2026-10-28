//! `orch.cancel`: stops a run that is going, by event, and leaves the cards ready for another run.
mod common;
use axelstra_daemon::client::Client;
use axelstra_daemon::server;
use serde_json::json;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

fn sh(cwd: &Path, args: &[&str]) {
    assert!(Command::new("git").current_dir(cwd).args(args).status().unwrap().success(), "git {args:?}");
}

#[test]
fn cancel_stops_the_run_kills_the_worker_and_puts_the_card_back_to_todo() {
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(120));
        eprintln!("watchdog: test stuck");
        std::process::exit(1);
    });
    let root = std::env::temp_dir().join(format!("axelstra-cancel-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let repo = root.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    sh(&repo, &["init", "-q", "-b", "main"]);
    sh(&repo, &["config", "user.email", "t@t"]);
    sh(&repo, &["config", "user.name", "t"]);
    std::fs::write(repo.join("shared.txt"), "base\n").unwrap();
    sh(&repo, &["add", "-A"]);
    sh(&repo, &["commit", "-qm", "init"]);

    let pipe = format!("{}-ocan-{}", common::base_pipe(), std::process::id());
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
    let pid = c.call("project.add", json!({"path": &repo})).unwrap()["id"].as_str().unwrap().to_string();
    let err = c.call("orch.cancel", json!({"project": pid})).unwrap_err();
    assert!(format!("{err:#}").contains("no orchestration is running"), "{err:#}");

    c.call("kanban.save", json!({"project": pid, "title": "slow", "status": "todo"})).unwrap();
    // an agent that never reports: only a cancel ends the run
    let agent: Vec<String> = if cfg!(windows) {
        ["powershell", "-NoProfile", "-Command", "Start-Sleep -Seconds 600"].map(String::from).to_vec()
    } else {
        ["sh", "-c", "sleep 600"].map(String::from).to_vec()
    };
    let started = c.call("orch.start", json!({"project": pid, "agent": agent})).unwrap();
    let run = started["name"].as_str().unwrap().to_string();

    // wait until the worker is really running (the card moved to in-progress)
    let t0 = Instant::now();
    loop {
        let board = c.call("kanban.list", json!({"project": pid})).unwrap();
        if board["cards"].as_array().unwrap().iter().any(|x| x["meta"]["status"] == "in-progress") {
            break;
        }
        assert!(t0.elapsed() < Duration::from_secs(60), "the worker never started");
        std::thread::sleep(Duration::from_millis(200));
    }
    let r = c.call("orch.cancel", json!({"project": pid})).unwrap();
    assert_eq!(r["run"], run.as_str());

    let done = loop {
        let st = c.call("orch.status", json!({"project": pid})).unwrap();
        if st["state"] != "running" {
            break st;
        }
        assert!(t0.elapsed() < Duration::from_secs(100), "the run did not stop");
        std::thread::sleep(Duration::from_millis(200));
    };
    assert_eq!(done["state"], "done", "{done}");
    assert_eq!(done["cancelled"], true, "{done}");
    assert_eq!(done["merged"], json!([]), "{done}");
    let board = c.call("kanban.list", json!({"project": pid})).unwrap();
    let card = board["cards"].as_array().unwrap().iter().find(|x| x["meta"]["id"] == "slow").unwrap();
    assert_eq!(card["meta"]["status"], "todo", "a later run picks it up again");
    // the worker's session is gone and nothing else is running for the project
    let live = c.call("session.list", json!({})).unwrap();
    assert!(live.as_array().unwrap().iter().all(|s| s["running"] != true), "{live}");
    // nothing left to stop; a new run can start
    assert!(c.call("orch.cancel", json!({"project": pid})).is_err());
}
