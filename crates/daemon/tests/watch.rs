use axelstra_daemon::client::{Client, Frame};
use axelstra_daemon::server;
use serde_json::json;
use std::path::Path;
use std::process::Command;
use std::sync::mpsc;
use std::time::Duration;

fn sh(cwd: &Path, args: &[&str]) {
    assert!(Command::new("git").current_dir(cwd).args(args).status().unwrap().success(), "git {args:?}");
}

#[test]
fn change_events_are_filtered_and_debounced() {
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(90));
        eprintln!("watchdog: test stuck");
        std::process::exit(1);
    });

    let root = std::env::temp_dir().join(format!("axelstra-watch-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let repo = root.join("r");
    std::fs::create_dir_all(repo.join("node_modules")).unwrap();
    sh(&repo, &["init", "-q", "-b", "main"]);
    sh(&repo, &["config", "user.email", "t@t"]);
    sh(&repo, &["config", "user.name", "t"]);
    std::fs::write(repo.join(".gitignore"), "*.log\n").unwrap();
    std::fs::write(repo.join("a.txt"), "x").unwrap();
    sh(&repo, &["add", "-A"]);
    sh(&repo, &["commit", "-qm", "init"]);

    let pipe = format!("{}-watch-{}", axelstra_daemon::proto::pipe_name(), std::process::id());
    let token = "w".repeat(32);
    let (p, t, s) = (pipe.clone(), token.clone(), root.join("projects.json"));
    std::thread::spawn(move || {
        tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap()
            .block_on(server::serve_with_store(p, t, s)).unwrap();
    });
    std::thread::sleep(Duration::from_millis(300));
    let mut c = Client::connect(&pipe, &token).unwrap();
    c.call("project.add", json!({"path": &repo})).unwrap();
    let wt = c.call("project.list", json!({})).unwrap()[0]["worktrees"][0]["path"].clone();

    // a second connection only listens
    let mut w = Client::connect(&pipe, &token).unwrap();
    w.call("watch.subscribe", json!({"worktree": wt})).unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        while let Ok(f) = w.read() {
            if let Frame::Event(e) = f {
                let _ = tx.send(e);
            }
        }
    });

    // quiet by default
    assert!(rx.recv_timeout(Duration::from_millis(1500)).is_err(), "idle worktree must not emit");

    // noise that git never shows: ignored dirs and gitignored files
    std::fs::write(repo.join("node_modules/pkg.js"), "x").unwrap();
    std::fs::write(repo.join("debug.log"), "x").unwrap();
    assert!(rx.recv_timeout(Duration::from_millis(1500)).is_err(), "ignored paths must not emit");

    // a burst of real edits gives one event
    for i in 0..10 {
        std::fs::write(repo.join("a.txt"), format!("v{i}")).unwrap();
    }
    let ev = rx.recv_timeout(Duration::from_secs(5)).expect("edit must emit");
    assert_eq!(ev["event"], "fs.changed");
    // a slow runner can split the burst across the debounce window once or twice; ten events would be no coalescing
    let mut extra = 0;
    while rx.recv_timeout(Duration::from_millis(1200)).is_ok() {
        extra += 1;
        assert!(extra <= 2, "burst must be coalesced, got {extra} extra events");
    }

    // git state changes emit too (HEAD/index moved by a commit)
    sh(&repo, &["add", "-A"]);
    sh(&repo, &["commit", "-qm", "two"]);
    rx.recv_timeout(Duration::from_secs(5)).expect("commit must emit");

    // reading changes through the daemon must not feed back into new events
    let _ = rx.try_iter().count();
    std::thread::sleep(Duration::from_millis(1500));
    let _ = rx.try_iter().count();
    c.call("git.changes", json!({"worktree": wt})).unwrap();
    assert!(rx.recv_timeout(Duration::from_millis(2500)).is_err(), "git.changes must not trigger the watcher");
}
