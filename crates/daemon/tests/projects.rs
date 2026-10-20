
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

    // a worktree made by another tool is not listed
    sh(&repo, &["worktree", "add", "-q", "-b", "foreign", &root.join("foreign-wt").to_string_lossy(), "HEAD"]);
    let list = c.call("project.list", json!({})).unwrap();
    assert_eq!(list[0]["worktrees"].as_array().unwrap().len(), 2);
    // name and logo come from the origin remote, like Orca; credentials never leak
    assert_eq!(list[0]["name"], "widgets");
    assert_eq!(list[0]["slug"], "Acme/widgets");
    assert_eq!(list[0]["icon_url"], "https://github.com/Acme.png?size=64");
    assert!(!list[0]["remote"].as_str().unwrap().contains("ghp_secret"));
    assert_eq!(list[0]["worktrees"][0]["is_main"], true);

    // a logo file in the repo wins over the host avatar
    assert!(c.call("project.logo", json!({"id": id})).unwrap()["data_url"].is_null());
    std::fs::create_dir_all(repo.join("assets")).unwrap();
    std::fs::write(repo.join("assets/logo.png"), [137u8, 80, 78, 71]).unwrap();
    let logo = c.call("project.logo", json!({"id": id})).unwrap();
    assert!(logo["data_url"].as_str().unwrap().starts_with("data:image/png;base64,"));

    // a session carries its project/worktree so the UI can group it
    let sid = c
        .call(
            "session.spawn",
            json!({"program": if cfg!(windows) { "cmd" } else { "sh" }, "args": if cfg!(windows) { vec!["/q"] } else { vec![] }, "cwd": wt["path"], "label": "shell",
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

#[test]
fn worktree_from_pull_request_head() {
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(90));
        eprintln!("watchdog: test stuck");
        std::process::exit(1);
    });
    let root = std::env::temp_dir().join(format!("axelstra-prwt-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let remote = root.join("remote.git");
    let repo = root.join("repo");
    std::fs::create_dir_all(&remote).unwrap();
    std::fs::create_dir_all(&repo).unwrap();
    sh(&remote, &["init", "-q", "--bare", "-b", "main"]);
    sh(&repo, &["init", "-q", "-b", "main"]);
    sh(&repo, &["config", "user.email", "t@t"]);
    sh(&repo, &["config", "user.name", "t"]);
    std::fs::write(repo.join("a.txt"), "base").unwrap();
    sh(&repo, &["add", "-A"]);
    sh(&repo, &["commit", "-qm", "init"]);
    sh(&repo, &["remote", "add", "origin", &remote.to_string_lossy()]);
    // a PR head lives under refs/pull/<n>/head on the host
    sh(&repo, &["checkout", "-q", "-b", "feature"]);
    std::fs::write(repo.join("pr.txt"), "from the pr").unwrap();
    sh(&repo, &["add", "-A"]);
    sh(&repo, &["commit", "-qm", "pr work"]);
    sh(&repo, &["push", "-q", "origin", "main", "HEAD:refs/pull/1/head"]);
    sh(&repo, &["checkout", "-q", "main"]);
    sh(&repo, &["branch", "-D", "feature"]);

    let pipe = format!("{}-prwt-{}", axelstra_daemon::proto::pipe_name(), std::process::id());
    let token = "p".repeat(32);
    let (p, t, s) = (pipe.clone(), token.clone(), root.join("projects.json"));
    std::thread::spawn(move || {
        tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap()
            .block_on(server::serve_with_store(p, t, s)).unwrap();
    });
    std::thread::sleep(Duration::from_millis(300));
    let mut c = Client::connect(&pipe, &token).unwrap();
    let id = c.call("project.add", json!({"path": &repo})).unwrap()["id"].as_str().unwrap().to_string();
    let wt = c.call("worktree.from_pr", json!({"project": id, "number": 1})).unwrap();
    assert_eq!(wt["branch"], "ax/pr-1");
    assert_eq!(std::fs::read_to_string(Path::new(wt["path"].as_str().unwrap()).join("pr.txt")).unwrap(), "from the pr");
    assert!(c.call("worktree.from_pr", json!({"project": id, "number": 99})).is_err(), "unknown PR");
}
