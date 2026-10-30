//! The main checkout never has tasks, it only dispatches them: an agent connection or the CLI (`from: "cli"`) cannot
//! start a task (an agent that is not the project main, a worker) in a project's own folder (`session.main_no_tasks`).
//! Terminals and the project main stay allowed there, tasks in a worktree too, and the app is not refused.
mod common;
use axelstra_daemon::client::Client;
use axelstra_daemon::server;
use serde_json::{json, Value};
use std::time::{Duration, Instant};

fn shell() -> (&'static str, Value) {
    if cfg!(windows) {
        ("cmd", json!(["/q", "/k", "echo TOK[%AXELSTRA_AGENT_TOKEN%]"]))
    } else {
        ("sh", json!(["-c", "echo TOK[$AXELSTRA_AGENT_TOKEN]; exec cat"]))
    }
}

fn code(e: &anyhow::Error) -> Option<String> {
    axelstra_core::msg::find(e).map(|c| c.code.clone())
}

#[test]
fn the_main_checkout_takes_no_tasks_from_agents_or_the_cli() {
    let store = common::store("mainnotasks");
    let root = store.parent().unwrap().to_path_buf();
    let dir = |n: &str| {
        let d = root.join(n);
        std::fs::create_dir_all(&d).unwrap();
        d.to_string_lossy().replace('\\', "/")
    };
    let repo = dir("app");
    // app-fix is a real git worktree of the project, so the janitor's orphan sweep keeps it
    let git = |args: &[&str]| {
        let ok = std::process::Command::new("git").current_dir(&repo).args(args).status().unwrap().success();
        assert!(ok, "git {args:?}");
    };
    git(&["init", "-q", "-b", "main"]);
    git(&["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "--allow-empty", "-m", "init"]);
    let wt = root.join(".axelstra-wt/app-fix").to_string_lossy().replace('\\', "/");
    git(&["worktree", "add", "-q", "-b", "fix", &wt]);
    std::fs::write(&store, json!([{"id": "p1", "name": "app", "path": repo}]).to_string()).unwrap();
    let pipe = common::pipe("mainnotasks");
    let token = "c".repeat(32);
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
    common::wait_ready(&pipe, &token);
    let mut app = Client::connect(&pipe, &token).unwrap();
    let (prog, args) = shell();
    let req = |cwd: &str, meta: Value, extra: Value| {
        let mut r = json!({"program": prog, "args": args, "cwd": cwd, "meta": meta});
        for (k, v) in extra.as_object().cloned().unwrap_or_default() {
            r[k] = v;
        }
        r
    };
    let agent_meta = |w: &str| json!({"project": "p1", "worktree": w, "kind": "agent", "agent": "claude"});
    let main_meta = json!({"project": "p1", "worktree": repo, "kind": "agent", "agent": "claude", "main": true});

    // the app starts the project main in the checkout; its credential gives an agent connection
    let main = app.call("session.spawn", req(&repo, main_meta.clone(), json!({}))).unwrap()["id"].as_u64().unwrap();
    let t0 = Instant::now();
    let agent_token = loop {
        let lines = app.call("session.screen", json!({"id": main, "lines": 100})).unwrap()["lines"].to_string();
        if let Some(rest) = lines.find("TOK[").map(|a| lines[a + 4..].to_string()) {
            if let Some(b) = rest.find(']').filter(|b| *b >= 32) {
                break rest[..b].to_string();
            }
        }
        assert!(t0.elapsed() < Duration::from_secs(15), "no token printed: {lines}");
        std::thread::sleep(Duration::from_millis(200));
    };
    let mut agent = Client::connect_agent(&pipe, &"x".repeat(32), &agent_token).unwrap();

    // refused: a task, a worker, a lead and a second "main" from the agent; a task from the CLI; a folder below the checkout
    let below = format!("{repo}/src");
    std::fs::create_dir_all(&below).unwrap();
    for (who, r) in [
        ("agent task", req(&repo, agent_meta(&repo), json!({}))),
        ("agent worker", req(&repo, agent_meta(&repo), json!({"worker": true, "reap": {"mode": "exit", "parent": main}}))),
        ("agent lead", req(&repo, json!({"project": "p1", "worktree": repo, "kind": "agent", "lead": true}), json!({}))),
        ("second main", req(&repo, main_meta.clone(), json!({}))),
        ("agent below", req(&below, json!({"kind": "agent", "agent": "claude"}), json!({}))),
    ] {
        let e = agent.call("session.spawn", r).unwrap_err();
        assert_eq!(code(&e).as_deref(), Some("session.main_no_tasks"), "{who}: {e:#}");
        assert!(e.to_string().contains("axelstra dispatch new app <name>"), "{who}: {e}");
    }
    let e = app.call("session.spawn", req(&repo, agent_meta(&repo), json!({"from": "cli"}))).unwrap_err();
    assert_eq!(code(&e).as_deref(), Some("session.main_no_tasks"), "cli task: {e:#}");

    // allowed: a terminal from the agent and the CLI, a task in a worktree, and the app (it steers the user itself)
    let term = json!({"project": "p1", "worktree": repo, "kind": "terminal"});
    let mut started = vec![
        agent.call("session.spawn", req(&repo, term.clone(), json!({}))).unwrap(),
        app.call("session.spawn", req(&repo, term, json!({"from": "cli"}))).unwrap(),
        agent.call("session.spawn", req(&wt, agent_meta(&wt), json!({}))).unwrap(),
        app.call("session.spawn", req(&wt, agent_meta(&wt), json!({"from": "cli"}))).unwrap(),
        app.call("session.spawn", req(&repo, agent_meta(&repo), json!({}))).unwrap(),
    ];
    started.push(json!({"id": main}));
    for s in started {
        let id = s["id"].as_u64().unwrap_or_else(|| panic!("not started: {s}"));
        let _ = app.call("session.kill", json!({"id": id}));
    }
}

#[test]
fn the_project_main_starts_from_the_cli_when_none_runs() {
    let store = common::store("mainfirst");
    let repo = store.parent().unwrap().join("app");
    std::fs::create_dir_all(&repo).unwrap();
    let repo = repo.to_string_lossy().replace('\\', "/");
    std::fs::write(&store, json!([{"id": "p1", "name": "app", "path": repo}]).to_string()).unwrap();
    let pipe = common::pipe("mainfirst");
    let token = "b".repeat(32);
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
    common::wait_ready(&pipe, &token);
    let mut app = Client::connect(&pipe, &token).unwrap();
    let (prog, args) = shell();
    let meta = json!({"project": "p1", "worktree": repo, "kind": "agent", "agent": "claude", "main": true});
    let r = app.call("session.spawn", json!({"program": prog, "args": args, "cwd": repo, "meta": meta, "from": "cli"})).unwrap();
    let _ = app.call("session.kill", json!({"id": r["id"]}));
}
