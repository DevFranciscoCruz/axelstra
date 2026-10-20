#![cfg(windows)]

use axelstra_daemon::client::Client;
use axelstra_daemon::server;
use serde_json::json;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

fn sh(cwd: &Path, args: &[&str]) {
    assert!(Command::new("git").current_dir(cwd).args(args).status().unwrap().success(), "git {args:?}");
}

#[test]
fn projects_worktrees_and_session_meta() {
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(90));
        eprintln!("watchdog: test stuck");
        std::process::exit(1);
    });

    let root = std::env::temp_dir().join(format!("axelstra-proj-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let repo = root.join("myrepo");
    std::fs::create_dir_all(&repo).unwrap();
    sh(&repo, &["init", "-q", "-b", "main"]);
    sh(&repo, &["config", "user.email", "t@t"]);
    sh(&repo, &["config", "user.name", "t"]);
    std::fs::write(repo.join("a.txt"), "x").unwrap();
    sh(&repo, &["add", "-A"]);
    sh(&repo, &["commit", "-qm", "init"]);
    sh(&repo, &["remote", "add", "origin", "https://ghp_secret@github.com/Acme/widgets.git"]);

    let pipe = format!("{}-proj-{}", axelstra_daemon::proto::pipe_name(), std::process::id());
    let token = "p".repeat(32);
    let store = root.join("projects.json");
    let (p, t, s) = (pipe.clone(), token.clone(), store.clone());
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

    // a plain folder is rejected
    let plain = root.join("plain");
    std::fs::create_dir_all(&plain).unwrap();
    assert!(c.call("project.add", json!({"path": plain})).is_err());

    // adding from a subfolder resolves to the repo root; adding twice is idempotent
    std::fs::create_dir_all(repo.join("sub")).unwrap();
    let a = c.call("project.add", json!({"path": repo.join("sub")})).unwrap();
    let b = c.call("project.add", json!({"path": &repo})).unwrap();
    assert_eq!(a["id"], b["id"]);
    let id = a["id"].as_str().unwrap().to_string();

    let wt = c.call("worktree.create", json!({"project": id, "name": "feat x"})).unwrap();
    assert_eq!(wt["branch"], "ax/feat-x");
    assert!(Path::new(wt["path"].as_str().unwrap()).join("a.txt").exists());

    let list = c.call("project.list", json!({})).unwrap();
    assert_eq!(list[0]["worktrees"].as_array().unwrap().len(), 2);
    // name and logo come from the origin remote, like Orca; credentials never leak
    assert_eq!(list[0]["name"], "widgets");
    assert_eq!(list[0]["slug"], "Acme/widgets");
    assert_eq!(list[0]["icon_url"], "https://github.com/Acme.png?size=64");
    assert!(!list[0]["remote"].as_str().unwrap().contains("ghp_secret"));
    assert_eq!(list[0]["worktrees"][0]["is_main"], true);

    // a session carries its project/worktree so the UI can group it
    let sid = c
        .call(
            "session.spawn",
            json!({"program": "cmd", "args": ["/q"], "cwd": wt["path"], "label": "shell",
                   "meta": {"project": id, "worktree": wt["path"]}}),
        )
        .unwrap()["id"]
        .as_u64()
        .unwrap();
    let sessions = c.call("session.list", json!({})).unwrap();
    assert_eq!(sessions[0]["meta"]["project"], id.as_str());
    c.call("session.remove", json!({"id": sid})).unwrap();

    // the main worktree is protected, others can be removed (branch goes too)
    assert!(c.call("worktree.remove", json!({"project": id, "path": list[0]["worktrees"][0]["path"]})).is_err());
    c.call("worktree.remove", json!({"project": id, "path": wt["path"]})).unwrap();
    let list = c.call("project.list", json!({})).unwrap();
    assert_eq!(list[0]["worktrees"].as_array().unwrap().len(), 1);

    // survives a restart: state is on disk
    assert!(store.exists());
    c.call("project.remove", json!({"id": id})).unwrap();
    assert_eq!(c.call("project.list", json!({})).unwrap(), json!([]));
}
