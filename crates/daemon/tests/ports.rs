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
fn finds_the_dev_server_of_a_worktree() {
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(90));
        eprintln!("watchdog: test stuck");
        std::process::exit(1);
    });

    let root = std::env::temp_dir().join(format!("axelstra-ports-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let repo = root.join("r");
    std::fs::create_dir_all(&repo).unwrap();
    sh(&repo, &["init", "-q", "-b", "main"]);
    sh(&repo, &["config", "user.email", "t@t"]);
    sh(&repo, &["config", "user.name", "t"]);
    std::fs::write(repo.join("a.txt"), "x").unwrap();
    sh(&repo, &["add", "-A"]);
    sh(&repo, &["commit", "-qm", "init"]);

    let pipe = format!("{}-ports-{}", axelstra_daemon::proto::pipe_name(), std::process::id());
    let token = "o".repeat(32);
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
    c.call("project.add", json!({"path": &repo})).unwrap();
    let wt = c.call("project.list", json!({})).unwrap()[0]["worktrees"][0]["path"].clone();

    // nothing runs yet
    assert_eq!(c.call("ports.list", json!({"worktree": wt})).unwrap()["ports"], json!([]));

    // a shell that starts a listener: the port belongs to a child of the session, not to the session itself
    let (program, args): (&str, Vec<&str>) = if cfg!(windows) {
        (
            "powershell",
            vec!["-NoProfile", "-Command", "$l=[Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback,0);$l.Start();Start-Sleep 40"],
        )
    } else {
        ("python3", vec!["-c", "import socket,time;s=socket.socket();s.bind(('127.0.0.1',0));s.listen();time.sleep(40)"])
    };
    let id = c
        .call(
            "session.spawn",
            json!({"program": program, "args": args, "cwd": wt, "meta": {"worktree": wt, "kind": "shell"}, "cols": 80, "rows": 24}),
        )
        .unwrap()["id"]
        .clone();

    let end = Instant::now() + Duration::from_secs(20);
    let mut found = 0;
    while Instant::now() < end {
        let r = c.call("ports.list", json!({"worktree": wt})).unwrap();
        if let Some(p) = r["ports"].as_array().and_then(|a| a.first()) {
            found = p["port"].as_u64().unwrap();
            break;
        }
        std::thread::sleep(Duration::from_millis(300));
    }
    assert!(found > 0, "listener port not detected");

    c.call("session.kill", json!({"id": id})).unwrap();
    assert!(c.call("ports.list", json!({"worktree": root.to_string_lossy()})).is_err(), "only registered worktrees");
}
