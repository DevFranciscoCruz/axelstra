
use axelstra_daemon::client::Client;
use axelstra_daemon::server;
use serde_json::json;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

fn sh(cwd: &Path, args: &[&str]) {
    assert!(Command::new("git").current_dir(cwd).args(args).status().unwrap().success(), "git {args:?}");
}

/// What the UI does: cards from kanban.save, then orch.start, then poll orch.status.
#[test]
fn ui_flow_cards_to_merged_branch() {
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(120));
        eprintln!("watchdog: test stuck");
        std::process::exit(1);
    });

    let root = std::env::temp_dir().join(format!("axelstra-ui-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let repo = root.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    sh(&repo, &["init", "-q", "-b", "main"]);
    sh(&repo, &["config", "user.email", "t@t"]);
    sh(&repo, &["config", "user.name", "t"]);
    std::fs::write(repo.join("shared.txt"), "base\n").unwrap();
    sh(&repo, &["add", "-A"]);
    sh(&repo, &["commit", "-qm", "init"]);

    let pipe = format!("{}-oui-{}", axelstra_daemon::proto::pipe_name(), std::process::id());
    let token = "o".repeat(32);
    let (p, t, s) = (pipe.clone(), token.clone(), root.join("projects.json"));
    std::thread::spawn(move || {
        tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap()
            .block_on(server::serve_with_store(p, t, s)).unwrap();
    });
    std::thread::sleep(Duration::from_millis(300));
    let mut c = Client::connect(&pipe, &token).unwrap();
    let pid = c.call("project.add", json!({"path": &repo})).unwrap()["id"].as_str().unwrap().to_string();

    assert_eq!(c.call("orch.status", json!({"project": pid})).unwrap()["state"], "idle");
    // nothing in "todo": the run fails cleanly instead of hanging
    c.call("kanban.save", json!({"project": pid, "title": "alpha", "status": "todo"})).unwrap();
    c.call("kanban.save", json!({"project": pid, "title": "beta", "status": "todo", "depends_on": ["alpha"]})).unwrap();
    c.call("kanban.save", json!({"project": pid, "title": "idea only", "status": "backlog"})).unwrap();

    let tools = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools");
    let agent: Vec<String> = if cfg!(windows) {
        let script = axelstra_core::git::canon(&tools.join("fake-agent.ps1")).unwrap();
        ["powershell", "-NoProfile", "-File", &script.to_string_lossy()].map(String::from).to_vec()
    } else {
        let script = axelstra_core::git::canon(&tools.join("fake-agent.sh")).unwrap();
        ["sh", &script.to_string_lossy()].map(String::from).to_vec()
    };
    let r = c
        .call(
            "orch.start",
            json!({"project": pid, "agent": agent}),
        )
        .unwrap();
    assert_eq!(r["state"], "running");
    assert!(c.call("orch.start", json!({"project": pid})).is_err(), "second run while one is active");

    let t0 = Instant::now();
    let done = loop {
        let st = c.call("orch.status", json!({"project": pid})).unwrap();
        if st["state"] != "running" {
            break st;
        }
        assert!(t0.elapsed() < Duration::from_secs(100), "orchestration did not finish");
        std::thread::sleep(Duration::from_millis(300));
    };
    assert_eq!(done["state"], "done", "{done}");
    assert_eq!(done["merged"], json!(["alpha", "beta"]), "{done}");
    // backlog ideas are never dispatched
    let board = c.call("kanban.list", json!({"project": pid})).unwrap();
    let idea = board["cards"].as_array().unwrap().iter().find(|x| x["meta"]["id"] == "idea-only").unwrap();
    assert_eq!(idea["meta"]["status"], "backlog");
    let wt = done["integ_wt"].as_str().unwrap();
    assert!(Path::new(wt).join("alpha.txt").exists());
}
