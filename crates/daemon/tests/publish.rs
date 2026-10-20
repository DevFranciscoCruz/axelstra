
use axelstra_daemon::client::Client;
use axelstra_daemon::server;
use serde_json::json;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

fn sh(cwd: &Path, args: &[&str]) -> String {
    let o = Command::new("git").current_dir(cwd).args(args).output().unwrap();
    assert!(o.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&o.stderr));
    String::from_utf8_lossy(&o.stdout).trim().to_string()
}

#[test]
fn commit_and_push_to_local_remote() {
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(90));
        eprintln!("watchdog: test stuck");
        std::process::exit(1);
    });

    let root = std::env::temp_dir().join(format!("axelstra-pub-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let remote = root.join("remote.git");
    let repo = root.join("repo");
    std::fs::create_dir_all(&remote).unwrap();
    std::fs::create_dir_all(&repo).unwrap();
    sh(&remote, &["init", "-q", "--bare", "-b", "main"]);
    sh(&repo, &["init", "-q", "-b", "main"]);
    sh(&repo, &["config", "user.email", "me@example.com"]);
    sh(&repo, &["config", "user.name", "Me"]);
    std::fs::write(repo.join("a.txt"), "x").unwrap();
    sh(&repo, &["add", "-A"]);
    sh(&repo, &["commit", "-qm", "init"]);
    sh(&repo, &["remote", "add", "origin", &remote.to_string_lossy()]);

    let pipe = format!("{}-pub-{}", axelstra_daemon::proto::pipe_name(), std::process::id());
    let token = "u".repeat(32);
    let (p, t, s) = (pipe.clone(), token.clone(), root.join("projects.json"));
    std::thread::spawn(move || {
        tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap()
            .block_on(server::serve_with_store(p, t, s)).unwrap();
    });
    std::thread::sleep(Duration::from_millis(300));
    let mut c = Client::connect(&pipe, &token).unwrap();
    c.call("project.add", json!({"path": &repo})).unwrap();
    let wt = c.call("project.list", json!({})).unwrap()[0]["worktrees"][0]["path"].clone();

    // nothing changed yet
    assert!(c.call("git.commit", json!({"worktree": wt, "message": "x"})).is_err());

    std::fs::write(repo.join("b.txt"), "new").unwrap();
    assert!(c.call("git.commit", json!({"worktree": wt, "message": "  "})).is_err(), "empty message");
    let r = c.call("git.commit", json!({"worktree": wt, "message": "add b"})).unwrap();
    assert!(r["sha"].as_str().unwrap().len() >= 7);
    assert_eq!(sh(&repo, &["log", "-1", "--format=%an <%ae>|%s"]), "Me <me@example.com>|add b");

    c.call("git.push", json!({"worktree": wt})).unwrap();
    assert_eq!(sh(&remote, &["rev-parse", "main"]), sh(&repo, &["rev-parse", "HEAD"]));

    // the PR title is validated before anything is sent anywhere
    assert!(c.call("git.pr", json!({"worktree": wt, "title": " ", "body": ""})).is_err());
}
