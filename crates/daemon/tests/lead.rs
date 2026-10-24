//! The lead of a worktree: its first agent is named "main", takes no "Tarefa N" number, reports to the project main
//! and is the default parent of every task started in that worktree. A restart keeps the roles.
mod common;
use axelstra_daemon::client::Client;
use axelstra_daemon::server;
use serde_json::{json, Value};
use std::path::Path;
use std::time::Duration;

fn shell() -> (String, Vec<String>) {
    if cfg!(windows) {
        ("cmd".into(), vec!["/q".into()])
    } else {
        ("sh".into(), vec![])
    }
}

fn start(root: &Path, tag: &str) -> Client {
    std::fs::create_dir_all(root).unwrap();
    let pipe = format!("{}-lead-{tag}-{}", common::base_pipe(), std::process::id());
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
    Client::connect(&pipe, &token).unwrap()
}

fn spawn(c: &mut Client, wt: &str, label: &str, extra: Value) -> u64 {
    let (prog, args) = shell();
    let mut p = json!({"program": prog, "args": args, "label": label, "meta": {"project": "pl", "worktree": wt, "kind": "agent", "agent": "claude"}});
    for (k, v) in extra.as_object().unwrap() {
        if k == "meta" {
            for (mk, mv) in v.as_object().unwrap() {
                p["meta"][mk] = mv.clone();
            }
        } else {
            p[k] = v.clone();
        }
    }
    c.call("session.spawn", p).unwrap()["id"].as_u64().unwrap()
}

fn info(c: &mut Client, id: u64) -> Value {
    c.call("session.list", json!({})).unwrap().as_array().unwrap().iter().find(|s| s["id"] == id).unwrap().clone()
}

#[test]
fn the_first_agent_of_a_worktree_leads_and_tasks_report_to_it() {
    let root = std::env::temp_dir().join(format!("axelstra-lead-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let mut c = start(&root, "a");
    let main = spawn(&mut c, "C:/r/main", "claude", json!({"meta": {"main": true}}));
    let lead = spawn(&mut c, "C:/r/wt-a", "claude", json!({}));
    let t1 = spawn(&mut c, "C:/r/wt-a", "claude", json!({}));
    let t2 = spawn(&mut c, "C:/r/wt-a", "claude", json!({}));
    let other = spawn(&mut c, "C:/r/wt-b", "claude", json!({}));
    // a worker explicitly asked for in an empty worktree is a task, not its lead
    let w = spawn(&mut c, "C:/r/wt-c", "claude", json!({"worker": true}));

    let l = info(&mut c, lead);
    assert_eq!((l["label"].as_str(), l["meta"]["lead"].as_bool()), (Some("main"), Some(true)));
    assert_eq!(l["meta"]["parent"], main, "the lead reports to the project main");
    assert!(l["meta"]["note"].as_str().unwrap().contains("main - wt-a"));
    for (id, name) in [(t1, "Tarefa 1"), (t2, "Tarefa 2")] {
        let t = info(&mut c, id);
        assert_eq!(t["label"], name, "the lead takes no task number");
        assert!(t["meta"]["lead"].is_null());
        assert_eq!(t["meta"]["parent"], lead);
    }
    // the project main is untouched
    let m = info(&mut c, main);
    assert!(m["meta"]["lead"].is_null() && m["meta"]["parent"].is_null());
    // another worktree has its own lead
    assert_eq!(info(&mut c, other)["meta"]["lead"], true);
    assert!(info(&mut c, w)["meta"]["lead"].is_null());

    // sessions.json keeps the roles
    let saved: Value = serde_json::from_str(&std::fs::read_to_string(root.join("sessions.json")).unwrap()).unwrap();
    let specs = saved["data"].as_array().unwrap();
    let spec = |label: &str, wt: &str| specs.iter().find(|s| s["label"] == label && s["meta"]["worktree"] == wt).cloned().unwrap();
    assert_eq!(spec("main", "C:/r/wt-a")["meta"]["lead"], true);
    assert!(spec("Tarefa 1", "C:/r/wt-a")["meta"]["lead"].is_null());
}

#[test]
fn a_restart_restores_the_lead_and_old_sessions_stay_as_they_were() {
    let root = std::env::temp_dir().join(format!("axelstra-lead-restore-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    // saved by a daemon: a lead and its task, plus a session from before leads existed (a lone "Tarefa 1")
    let rec = |label: &str, wt: &str, extra: Value| {
        let (prog, args) = shell();
        let mut meta = json!({"project": "pl-restore", "worktree": wt, "kind": "agent", "agent": "claude"});
        for (k, v) in extra.as_object().unwrap() {
            meta[k] = v.clone();
        }
        json!({"program": prog, "args": args, "label": label, "meta": meta})
    };
    let saved = json!({"v": 1, "data": [
        rec("main", "C:/r/wt-a", json!({"lead": true})),
        rec("Tarefa 1", "C:/r/wt-a", json!({})),
        rec("Tarefa 1", "C:/r/old", json!({})),
    ]});
    std::fs::write(root.join("sessions.json"), saved.to_string()).unwrap();
    let mut c = start(&root, "b");
    let pending = c.call("session.pending", json!({})).unwrap();
    let mut ids = vec![];
    // the app replays them in the saved order, each as it was spawned
    for r in pending.as_array().unwrap() {
        ids.push(c.call("session.spawn", r.clone()).unwrap()["id"].as_u64().unwrap());
    }
    assert_eq!(ids.len(), 3);
    let lead = info(&mut c, ids[0]);
    let task = info(&mut c, ids[1]);
    let old = info(&mut c, ids[2]);
    assert!(task["meta"]["lead"].is_null(), "a named task is never promoted");
    assert_eq!((lead["label"].as_str(), lead["meta"]["lead"].as_bool()), (Some("main"), Some(true)));
    assert_eq!(task["meta"]["parent"], ids[0], "the task finds its lead again");
    assert_eq!((old["label"].as_str(), old["meta"]["lead"].as_bool()), (Some("Tarefa 1"), None));
}
