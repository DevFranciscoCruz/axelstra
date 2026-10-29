//! A session always starts in the folder it was asked for (never silently in the home folder), and a restart brings back
//! neither a task that already reported `done` nor anything but the lead of a new worktree.
mod common;
use axelstra_daemon::client::Client;
use axelstra_daemon::server;
use serde_json::{json, Value};
use std::path::Path;

fn shell() -> (String, Vec<String>) {
    if cfg!(windows) {
        ("cmd".into(), vec!["/q".into()])
    } else {
        ("sh".into(), vec![])
    }
}

fn start(root: &Path, tag: &str) -> Client {
    std::fs::create_dir_all(root).unwrap();
    let pipe = format!("{}-cwd-{tag}-{}", common::base_pipe(), std::process::id());
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
    common::wait_ready(&pipe, &token);
    Client::connect(&pipe, &token).unwrap()
}

fn agent(c: &mut Client, wt: &str, label: Option<&str>, cwd: &str) -> anyhow::Result<u64> {
    agent_of(c, "pc", wt, label, cwd)
}

/// Task numbers are counted per project name for the whole test process, so a test that asserts "Tarefa 1" owns its project.
fn agent_of(c: &mut Client, project: &str, wt: &str, label: Option<&str>, cwd: &str) -> anyhow::Result<u64> {
    let (prog, args) = shell();
    let mut p = json!({"program": prog, "args": args, "cwd": cwd, "meta": {"project": project, "worktree": wt, "kind": "agent", "agent": "claude"}});
    if let Some(l) = label {
        p["label"] = json!(l);
    }
    Ok(c.call("session.spawn", p)?["id"].as_u64().unwrap())
}

fn info(c: &mut Client, id: u64) -> Value {
    c.call("session.list", json!({})).unwrap().as_array().unwrap().iter().find(|s| s["id"] == id).unwrap().clone()
}

fn dir(root: &Path, n: &str) -> String {
    let d = root.join(n);
    std::fs::create_dir_all(&d).unwrap();
    d.to_string_lossy().replace('\\', "/")
}

#[test]
fn a_missing_cwd_is_an_error_not_the_home_folder() {
    let root = std::env::temp_dir().join(format!("axelstra-cwd-missing-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let wt = dir(&root, "wt");
    let mut c = start(&root, "a");
    let gone = root.join("deleted").to_string_lossy().replace('\\', "/");
    let before = c.call("session.list", json!({})).unwrap().as_array().unwrap().len();
    let e = agent(&mut c, &gone, None, &gone).unwrap_err();
    let coded = axelstra_core::msg::find(&e).expect("coded error");
    assert_eq!(coded.code, "session.cwd_missing");
    assert_eq!(c.call("session.list", json!({})).unwrap().as_array().unwrap().len(), before, "no session was left behind");
    // a file is no folder either
    let file = format!("{wt}/f.txt");
    std::fs::write(&file, "x").unwrap();
    assert_eq!(axelstra_core::msg::find(&agent(&mut c, &wt, None, &file).unwrap_err()).unwrap().code, "session.cwd_missing");
    // an existing folder starts
    let id = agent(&mut c, &wt, None, &wt).unwrap();
    assert!(info(&mut c, id)["running"].as_bool().unwrap_or(true));
}

#[test]
fn restoring_a_saved_session_of_an_existing_worktree_starts_there_and_a_vanished_one_is_dropped() {
    let root = std::env::temp_dir().join(format!("axelstra-cwd-restore-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let wt = dir(&root, "wt");
    let (prog, args) = shell();
    let rec = |w: &str| {
        json!({"program": prog, "args": args, "cwd": w, "label": "main",
               "meta": {"project": "pc", "worktree": w, "kind": "agent", "agent": "claude", "lead": true}})
    };
    let gone = root.join("gone").to_string_lossy().replace('\\', "/");
    std::fs::write(root.join("sessions.json"), json!({"v": 1, "data": [rec(&wt), rec(&gone)]}).to_string()).unwrap();
    let mut c = start(&root, "b");
    let pending = c.call("session.pending", json!({})).unwrap();
    let list = pending.as_array().unwrap();
    assert_eq!(list.len(), 1, "the record of a worktree that is gone is dropped");
    assert_eq!((list[0]["cwd"].as_str(), list[0]["meta"]["worktree"].as_str()), (Some(wt.as_str()), Some(wt.as_str())));
    let id = c.call("session.spawn", list[0].clone()).unwrap()["id"].as_u64().unwrap();
    assert_eq!(info(&mut c, id)["label"], "main");
}

#[test]
fn a_new_worktree_has_only_its_lead_and_a_finished_task_is_not_restored() {
    let root = std::env::temp_dir().join(format!("axelstra-cwd-done-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let wt = dir(&root, "wt-new");
    let mut c = start(&root, "c");
    // the "setup" tab is a shell: not an agent, no name, no number
    let (prog, args) = shell();
    let setup = c
        .call(
            "session.spawn",
            json!({"program": prog, "args": args, "cwd": wt, "label": "setup", "meta": {"project": "pc-done", "worktree": wt, "kind": "shell"}}),
        )
        .unwrap()["id"]
        .as_u64()
        .unwrap();
    let (prog, args) = shell();
    let lead = c
        .call(
            "session.spawn",
            json!({"program": prog, "args": args, "cwd": wt, "meta": {"project": "pc-done", "worktree": wt, "kind": "agent", "agent": "claude", "lead": true}}),
        )
        .unwrap()["id"]
        .as_u64()
        .unwrap();
    let l = info(&mut c, lead);
    assert_eq!((l["label"].as_str(), l["meta"]["lead"].as_bool()), (Some("main"), Some(true)));
    assert!(info(&mut c, setup)["meta"]["lead"].is_null());
    let sessions = c.call("session.list", json!({})).unwrap();
    assert!(
        !sessions.as_array().unwrap().iter().any(|s| s["label"].as_str().is_some_and(|l| l.starts_with("Tarefa"))),
        "no task exists until someone starts one"
    );
    // the lead starts a task; the task reports done
    let task = agent_of(&mut c, "pc-done", &wt, None, &wt).unwrap();
    assert_eq!(info(&mut c, task)["label"], "Tarefa 1");
    c.call("msg.send", json!({"from": task, "to": lead, "kind": "done", "text": "ok\nall green"})).unwrap();
    let data =
        common::saved_sessions_until(&root, |l| l.iter().any(|s| s["label"] == "main") && !l.iter().any(|s| s["label"] == "Tarefa 1"));
    let saved = json!({"v": 1, "data": data});
    let labels: Vec<&str> = saved["data"].as_array().unwrap().iter().filter_map(|s| s["label"].as_str()).collect();
    assert!(labels.contains(&"main") && !labels.contains(&"Tarefa 1"), "{labels:?}");
    // a file saved before the report was recorded (the mark is in the record) is not handed out either
    let mut old: Value = saved.clone();
    let rec = json!({"program": shell().0, "args": shell().1, "cwd": wt, "label": "Tarefa 9",
        "meta": {"project": "pc-done", "worktree": wt, "kind": "agent", "agent": "claude", "done": true}});
    old["data"].as_array_mut().unwrap().push(rec);
    let root2 = std::env::temp_dir().join(format!("axelstra-cwd-done2-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root2);
    std::fs::create_dir_all(&root2).unwrap();
    std::fs::write(root2.join("sessions.json"), old.to_string()).unwrap();
    let mut c2 = start(&root2, "d");
    let pending = c2.call("session.pending", json!({})).unwrap();
    let names: Vec<&str> =
        pending.as_array().unwrap().iter().filter(|s| s["meta"]["kind"] == "agent").filter_map(|s| s["label"].as_str()).collect();
    assert_eq!(names, vec!["main"], "of the agents only the lead comes back");
    // new work for a finished task makes it restorable again
    c.call("msg.send", json!({"from": lead, "to": task, "kind": "message", "text": "one more thing"})).unwrap();
    let data = common::saved_sessions_until(&root, |l| l.iter().any(|s| s["label"] == "Tarefa 1"));
    assert!(data.iter().any(|s| s["label"] == "Tarefa 1"));
}
