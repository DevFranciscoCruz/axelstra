mod common;
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

    let pipe = format!("{}-watch-{}", common::base_pipe(), std::process::id());
    let token = "w".repeat(32);
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

#[test]
fn worktree_created_by_another_client_is_announced() {
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(60));
        eprintln!("watchdog: test stuck");
        std::process::exit(1);
    });

    let root = std::env::temp_dir().join(format!("axelstra-wtev-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let repo = root.join("r");
    std::fs::create_dir_all(&repo).unwrap();
    sh(&repo, &["init", "-q", "-b", "main"]);
    sh(&repo, &["config", "user.email", "t@t"]);
    sh(&repo, &["config", "user.name", "t"]);
    std::fs::write(repo.join("a.txt"), "x").unwrap();
    sh(&repo, &["add", "-A"]);
    sh(&repo, &["commit", "-qm", "init"]);

    let pipe = format!("{}-wtev-{}", common::base_pipe(), std::process::id());
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
    let mut cli = Client::connect(&pipe, &token).unwrap();
    cli.call("project.add", json!({"path": &repo})).unwrap();
    let projects = cli.call("project.list", json!({})).unwrap();
    let (pid, wt) = (projects[0]["id"].clone(), projects[0]["worktrees"][0]["path"].clone());

    // the app's stream
    let mut app = Client::connect(&pipe, &token).unwrap();
    app.call("watch.subscribe", json!({"worktree": wt})).unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        while let Ok(f) = app.read() {
            if let Frame::Event(e) = f {
                let _ = tx.send(e);
            }
        }
    });

    // the CLI (a different connection) creates a worktree: the app hears about it without asking
    cli.call("worktree.create", json!({"project": pid, "name": "topic"})).unwrap();
    let mut got = false;
    while let Ok(e) = rx.recv_timeout(Duration::from_secs(5)) {
        if e["event"] == "projects.changed" {
            got = true;
            break;
        }
    }
    assert!(got, "worktree.create must emit projects.changed");
}

/// The app reads its event stream in one thread and sends `watch.subscribe` from another, on a duplicate of the same
/// connection, while the stream is quiet. On Windows a blocking read on the pipe would hold that write back until an
/// event came, so the subscription never reached the daemon and the app missed `projects.changed`.
#[test]
fn a_request_sent_on_a_quiet_event_stream_is_not_held_back_by_the_reader() {
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(60));
        eprintln!("watchdog: test stuck");
        std::process::exit(1);
    });

    let root = std::env::temp_dir().join(format!("axelstra-quiet-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let repo = root.join("r");
    std::fs::create_dir_all(&repo).unwrap();
    sh(&repo, &["init", "-q", "-b", "main"]);
    sh(&repo, &["config", "user.email", "t@t"]);
    sh(&repo, &["config", "user.name", "t"]);
    std::fs::write(repo.join("a.txt"), "x").unwrap();
    sh(&repo, &["add", "-A"]);
    sh(&repo, &["commit", "-qm", "init"]);

    let pipe = format!("{}-quiet-{}", common::base_pipe(), std::process::id());
    let token = "f".repeat(32);
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

    let mut app = Client::connect(&pipe, &token).unwrap();
    let mut rd = app.try_clone().unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        while let Ok(f) = rd.read_shared() {
            if let Frame::Event(e) = f {
                let _ = tx.send(e);
            }
        }
    });
    // let the reader settle into waiting before the request goes out
    std::thread::sleep(Duration::from_millis(300));
    app.send("watch.unsubscribe", json!({})).unwrap();
    std::thread::sleep(Duration::from_millis(500));

    let mut cli = Client::connect(&pipe, &token).unwrap();
    cli.call("project.add", json!({"path": &repo})).unwrap();
    let mut got = false;
    while let Ok(e) = rx.recv_timeout(Duration::from_secs(5)) {
        if e["event"] == "projects.changed" {
            got = true;
            break;
        }
    }
    assert!(got, "the app must hear projects.changed on a stream it only wrote one request to");
}
