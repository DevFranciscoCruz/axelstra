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

fn start(root: &Path, tag: &str) -> Client {
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(90));
        eprintln!("watchdog: test stuck");
        std::process::exit(1);
    });
    let pipe = format!("{}-bk-{tag}-{}", common::base_pipe(), std::process::id());
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
    common::wait_ready(&pipe, &token);
    Client::connect(&pipe, &token).unwrap()
}

fn fresh(tag: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!("axelstra-bk-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    root
}

#[test]
fn export_then_restore_brings_settings_back_and_leaves_the_token_out() {
    let root = fresh("roundtrip");
    std::fs::write(root.join("token"), "secret-token").unwrap();
    let mut c = start(&root, "roundtrip");
    c.call("settings.set", json!({"terse": "short", "notify_mode": "waiting"})).unwrap();

    let path = c.call("backup.export", json!({})).unwrap()["path"].as_str().unwrap().to_string();
    let files: Vec<String> = std::fs::read_dir(&path).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    assert!(files.contains(&"settings.json".to_string()), "{files:?}");
    assert!(files.iter().all(|f| !f.contains("token")), "{files:?}");

    c.call("settings.set", json!({"terse": "tiny", "notify_mode": "off"})).unwrap();
    let r = c.call("backup.restore", json!({"path": path})).unwrap();
    assert!(r["restored"].as_array().unwrap().iter().any(|f| f == "settings.json"));
    let s = c.call("settings.get", json!({})).unwrap();
    assert_eq!(s["terse"], "short");
    assert_eq!(s["notify_mode"], "waiting");
}

#[test]
fn restore_refuses_bad_input_and_keeps_current_state() {
    let root = fresh("bad");
    let mut c = start(&root, "bad");
    c.call("settings.set", json!({"terse": "short"})).unwrap();
    assert!(c.call("backup.restore", json!({})).is_err());
    assert!(c.call("backup.restore", json!({"path": root.join("missing")})).is_err());
    let bk = root.join("bk");
    std::fs::create_dir_all(&bk).unwrap();
    std::fs::write(bk.join("settings.json"), "{ nope").unwrap();
    let e = c.call("backup.restore", json!({"path": &bk})).unwrap_err().to_string();
    assert!(e.contains("unusable"), "{e}");
    assert_eq!(c.call("settings.get", json!({})).unwrap()["terse"], "short");
}

#[test]
fn env_tools_lists_git_claude_and_gh() {
    let root = fresh("env");
    let mut c = start(&root, "env");
    let tools = c.call("env.tools", json!({})).unwrap()["tools"].as_array().unwrap().clone();
    let ids: Vec<&str> = tools.iter().map(|t| t["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["git", "claude", "gh"]);
    // the test itself needs git, so it is found and reports a version
    assert_eq!(tools[0]["found"], true);
    assert!(tools[0]["version"].as_str().is_some_and(|v| v.contains('.')), "{:?}", tools[0]);
}

#[test]
fn trust_info_pins_the_todo_cards() {
    let root = fresh("trust");
    let repo = root.join("r");
    std::fs::create_dir_all(&repo).unwrap();
    sh(&repo, &["init", "-q", "-b", "main"]);
    sh(&repo, &["config", "user.email", "t@t"]);
    sh(&repo, &["config", "user.name", "t"]);
    std::fs::write(repo.join("a.txt"), "x").unwrap();
    sh(&repo, &["add", "-A"]);
    sh(&repo, &["commit", "-qm", "init"]);
    let mut c = start(&root, "trust");
    let pid = c.call("project.add", json!({"path": &repo})).unwrap()["id"].as_str().unwrap().to_string();
    std::fs::create_dir_all(repo.join(".kanban")).unwrap();
    let card = |extra: &str| format!("---\nid: a\nstatus: todo\n{extra}---\ntask a\n");
    std::fs::write(repo.join(".kanban/a.md"), card("permission_mode: bypassPermissions\nacceptance: \"echo ok\"\n")).unwrap();

    let t = c.call("orch.trust_info", json!({"project": pid})).unwrap();
    assert_eq!(t["cards"][0]["id"], "a");
    assert_eq!(t["cards"][0]["bypass"], true);
    assert_eq!(t["cards"][0]["permission_mode"], "bypassPermissions");
    assert_eq!(t["cards"][0]["acceptance"], "echo ok");
    let pin = t["pin"].as_str().unwrap().to_string();
    assert!(!pin.is_empty());

    // an edited card gets another pin, so a confirmation of the old one no longer matches
    std::fs::write(repo.join(".kanban/a.md"), card("permission_mode: bypassPermissions\nacceptance: \"echo changed\"\n")).unwrap();
    let t2 = c.call("orch.trust_info", json!({"project": pid})).unwrap();
    assert_ne!(t2["pin"].as_str().unwrap(), pin);
    assert!(c.call("orch.trust_info", json!({"project": "nope"})).is_err());
}
