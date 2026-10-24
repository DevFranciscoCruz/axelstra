mod common;
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
fn timeline_lists_the_commits_of_a_worktree_branch() {
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(90));
        eprintln!("watchdog: test stuck");
        std::process::exit(1);
    });
    let root = std::env::temp_dir().join(format!("axelstra-tl-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let repo = root.join("r");
    std::fs::create_dir_all(&repo).unwrap();
    sh(&repo, &["init", "-q", "-b", "main"]);
    sh(&repo, &["config", "user.email", "t@t"]);
    sh(&repo, &["config", "user.name", "t"]);
    std::fs::write(repo.join("a.txt"), "1").unwrap();
    sh(&repo, &["add", "-A"]);
    sh(&repo, &["commit", "-qm", "init"]);

    let pipe = format!("{}-tl-{}", axelstra_daemon::proto::pipe_name(), std::process::id());
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
    let mut c = Client::connect(&pipe, &token).unwrap();
    let pr = c.call("project.add", json!({"path": &repo})).unwrap();
    let wt = c.call("worktree.create", json!({"project": pr["id"], "name": "tl"})).unwrap();
    let wtp = wt["path"].as_str().unwrap().to_string();
    let wtp = Path::new(&wtp);
    std::fs::write(wtp.join("b.txt"), "2").unwrap();
    sh(wtp, &["add", "-A"]);
    sh(wtp, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qm", "add b"]);

    let t = c.call("git.timeline", json!({"worktree": wtp, "project": pr["id"]})).unwrap();
    assert_eq!(t["base"], "main");
    let events = t["events"].as_array().unwrap();
    assert!(events.iter().any(|e| e["kind"] == "commit" && e["title"] == "add b"), "{t}");
    assert!(t["ci"].is_null(), "CI is only asked for on request");

    // a path that is not a known worktree is refused
    assert!(c.call("git.timeline", json!({"worktree": root.join("nowhere"), "project": pr["id"]})).is_err());
    let _ = std::fs::remove_dir_all(&root);
}
