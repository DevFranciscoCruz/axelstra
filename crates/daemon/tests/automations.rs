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
fn save_run_and_delete() {
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(90));
        eprintln!("watchdog: test stuck");
        std::process::exit(1);
    });
    // never start a real agent from a test
    std::env::set_var("AXELSTRA_AGENT_CLI", "echo");

    let root = std::env::temp_dir().join(format!("axelstra-auto-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let repo = root.join("r");
    std::fs::create_dir_all(&repo).unwrap();
    sh(&repo, &["init", "-q", "-b", "main"]);
    sh(&repo, &["config", "user.email", "t@t"]);
    sh(&repo, &["config", "user.name", "t"]);
    std::fs::write(repo.join("a.txt"), "x").unwrap();
    sh(&repo, &["add", "-A"]);
    sh(&repo, &["commit", "-qm", "init"]);

    let pipe = format!("{}-auto-{}", axelstra_daemon::proto::pipe_name(), std::process::id());
    let token = "a".repeat(32);
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
    let pid = c.call("project.add", json!({"path": &repo})).unwrap()["id"].as_str().unwrap().to_string();

    let mk = |name: &str, schedule: serde_json::Value| json!({"id": "", "name": name, "project": pid, "agent": "claude", "prompt": "look at it's TODOs", "schedule": schedule, "enabled": true, "last_run": 0});

    // bad input is refused before anything is stored
    assert!(c.call("automation.save", mk("", json!({"kind": "daily", "time": "09:00"}))).is_err());
    assert!(c.call("automation.save", mk("x", json!({"kind": "interval", "minutes": 1}))).is_err());
    assert!(c.call("automation.save", mk("x", json!({"kind": "daily", "time": "99:99"}))).is_err());
    let mut bad = mk("x", json!({"kind": "daily", "time": "09:00"}));
    bad["project"] = json!("nope");
    assert!(c.call("automation.save", bad).is_err(), "unknown project");
    assert_eq!(c.call("automation.list", json!({})).unwrap()["automations"], json!([]));

    let a = c.call("automation.save", mk("Morning sweep", json!({"kind": "daily", "time": "09:00"}))).unwrap();
    let id = a["id"].as_str().unwrap().to_string();
    assert!(id.starts_with("auto-"));
    assert!(a["last_run"].as_i64().unwrap() > 0, "created now: nothing fires retroactively");
    assert_eq!(c.call("automation.list", json!({})).unwrap()["automations"].as_array().unwrap().len(), 1);

    // editing in place keeps the id
    let mut edited = a.clone();
    edited["name"] = json!("Morning sweep 2");
    let e = c.call("automation.save", edited).unwrap();
    assert_eq!(e["id"], a["id"]);
    assert_eq!(c.call("automation.list", json!({})).unwrap()["automations"].as_array().unwrap().len(), 1);

    // run now: a fresh worktree and an agent session started with the prompt
    let r = c.call("automation.run", json!({"id": id})).unwrap();
    let wt = r["worktree"].as_str().unwrap().to_string();
    assert!(Path::new(&wt).join("a.txt").exists());
    let sessions = c.call("session.list", json!({})).unwrap();
    let s = sessions.as_array().unwrap().iter().find(|s| s["id"] == r["session"]).expect("session listed");
    assert_eq!(s["label"], "Morning sweep 2");
    assert_eq!(s["meta"]["kind"], "agent");
    assert!(c.call("automation.run", json!({"id": "missing"})).is_err());

    c.call("automation.delete", json!({"id": id})).unwrap();
    assert_eq!(c.call("automation.list", json!({})).unwrap()["automations"], json!([]));
}
