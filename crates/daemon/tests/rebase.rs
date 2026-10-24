use axelstra_daemon::client::Client;
use axelstra_daemon::server;
use serde_json::json;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

fn git(cwd: &Path, args: &[&str]) {
    let o = Command::new("git").current_dir(cwd).args(["-c", "user.name=t", "-c", "user.email=t@t"]).args(args).output().unwrap();
    assert!(o.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&o.stderr));
}

#[test]
fn worktree_rebase_over_the_daemon() {
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(120));
        eprintln!("watchdog: test stuck");
        std::process::exit(1);
    });
    // the daemon's own git calls take their identity from the environment
    for (k, v) in [("GIT_AUTHOR_NAME", "t"), ("GIT_AUTHOR_EMAIL", "t@t"), ("GIT_COMMITTER_NAME", "t"), ("GIT_COMMITTER_EMAIL", "t@t")] {
        std::env::set_var(k, v);
    }
    let root = std::env::temp_dir().join(format!("axelstra-rebase-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let repo = root.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("a.txt"), "1\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "init"]);

    let pipe = format!("{}-rebase-{}", axelstra_daemon::proto::pipe_name(), std::process::id());
    let token = "b".repeat(32);
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
    let id = c.call("project.add", json!({"path": &repo})).unwrap()["id"].as_str().unwrap().to_string();
    let wt = c.call("worktree.create", json!({"project": id, "name": "topic"})).unwrap();
    let wpath = wt["path"].as_str().unwrap().to_string();
    let w = Path::new(&wpath);
    std::fs::write(w.join("b.txt"), "b").unwrap();
    git(w, &["add", "."]);
    git(w, &["commit", "-qm", "b"]);
    std::fs::write(repo.join("c.txt"), "c").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "c"]);
    let req = || json!({"project": id, "path": wpath});

    // the main worktree is never rebased
    let main_path = repo.to_string_lossy().to_string();
    assert!(c.call("worktree.rebase", json!({"project": id, "path": main_path})).is_err());

    // the app role may; an agent session may not
    let (prog, args) = if cfg!(windows) {
        ("cmd", json!(["/c", "echo TOK[%AXELSTRA_AGENT_TOKEN%] & ping -n 40 127.0.0.1"]))
    } else {
        ("sh", json!(["-c", "echo TOK[$AXELSTRA_AGENT_TOKEN]; i=0; while [ $i -lt 40 ]; do echo x; sleep 1; i=$((i+1)); done"]))
    };
    let sid = c
        .call("session.spawn", json!({"program": prog, "args": args, "cwd": wpath, "meta": {"project": id, "worktree": wpath}}))
        .unwrap()["id"]
        .as_u64()
        .unwrap();
    let t0 = Instant::now();
    let agent_token = loop {
        let lines = c.call("session.screen", json!({"id": sid})).unwrap()["lines"].to_string();
        if let Some(a) = lines.find("TOK[") {
            let rest = &lines[a + 4..];
            if let Some(b) = rest.find(']') {
                if b >= 32 {
                    break rest[..b].to_string();
                }
            }
        }
        assert!(t0.elapsed() < Duration::from_secs(15), "no token printed: {lines}");
        std::thread::sleep(Duration::from_millis(200));
    };
    let mut agent = Client::connect_agent(&pipe, &"x".repeat(32), &agent_token).unwrap();
    let e = agent.call("worktree.rebase", req()).unwrap_err().to_string();
    assert!(e.contains("not allowed to agents"), "{e}");

    // a session still printing in the worktree: refused, nothing moved
    let before = Command::new("git").current_dir(w).args(["rev-parse", "HEAD"]).output().unwrap().stdout;
    let e = c.call("worktree.rebase", req()).unwrap_err().to_string();
    assert!(e.contains("session is active"), "{e}");
    c.call("session.remove", json!({"id": sid})).unwrap();

    // dirty (an untracked file counts): refused
    std::fs::write(w.join("new.txt"), "x").unwrap();
    let e = c.call("worktree.rebase", req()).unwrap_err().to_string();
    assert!(e.contains("uncommitted changes"), "{e}");
    std::fs::remove_file(w.join("new.txt")).unwrap();
    assert_eq!(Command::new("git").current_dir(w).args(["rev-parse", "HEAD"]).output().unwrap().stdout, before);

    // clean and quiet: rebased onto main
    let r = c.call("worktree.rebase", req()).unwrap();
    assert_eq!(r["ok"], true, "{r}");
    assert_ne!(r["from"], r["to"]);
    assert!(w.join("c.txt").exists());

    // the overlap files method is open to the app and answers an object
    assert!(c.call("dispatch.overlap_files", json!({})).unwrap().is_object());
}
