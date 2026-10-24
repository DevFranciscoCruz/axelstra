//! Token budgets: the setting is validated and survives, a level crossing is reported once, an agent is interrupted
//! only for a budget that opted in, and an agent cannot touch the limits.
mod common;
use axelstra_daemon::agentlog::claude_dir_name;
use axelstra_daemon::client::Client;
use axelstra_daemon::server;
use serde_json::{json, Value};
use std::io::Write;
use std::path::Path;
use std::time::{Duration, Instant};

fn start(root: &Path, tag: &str) -> (String, String) {
    std::fs::create_dir_all(root).unwrap();
    let pipe = format!("{}-budget-{tag}-{}", axelstra_daemon::proto::pipe_name(), std::process::id());
    let token = "d".repeat(32);
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
    (pipe, token)
}

/// Agent logs are read from the user's home: point it at a temp folder (this file has one test that uses usage).
fn fake_home(root: &Path) -> std::path::PathBuf {
    let home = root.join("home");
    std::fs::create_dir_all(&home).unwrap();
    std::env::set_var("USERPROFILE", &home);
    std::env::set_var("HOME", &home);
    home
}

/// Appends a reply of `tokens` output tokens to the Claude transcript of `wt` (transcripts only ever grow).
fn log_tokens(home: &Path, wt: &str, id: &str, tokens: u64) {
    let dir = home.join(".claude").join("projects").join(claude_dir_name(wt));
    std::fs::create_dir_all(&dir).unwrap();
    let line = json!({"message": {"id": id, "usage": {"input_tokens": 0, "output_tokens": tokens}}});
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("s.jsonl")).unwrap();
    writeln!(f, "{line}").unwrap();
}

/// A session that keeps printing, so it counts as working.
fn spawn_working_agent(c: &mut Client, wt: &str) -> u64 {
    let (prog, args) = if cfg!(windows) {
        ("cmd", json!(["/c", "ping -n 120 127.0.0.1"]))
    } else {
        ("sh", json!(["-c", "while true; do echo x; sleep 1; done"]))
    };
    let meta = json!({"project": "p", "worktree": wt, "kind": "agent", "agent": "claude"});
    c.call("session.spawn", json!({"program": prog, "args": args, "meta": meta})).unwrap()["id"].as_u64().unwrap()
}

fn events(c: &mut Client) -> Vec<Value> {
    c.call("budget.events", json!({})).unwrap()["events"].as_array().cloned().unwrap()
}

#[test]
fn limits_alert_once_and_pause_only_on_opt_in() {
    let root = std::env::temp_dir().join(format!("axelstra-budget-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let home = fake_home(&root);
    let (pipe, token) = start(&root.join("d"), "main");
    let mut app = Client::connect(&pipe, &token).unwrap();
    let (a, b) = (root.join("wt-a").to_string_lossy().replace('\\', "/"), root.join("wt-b").to_string_lossy().replace('\\', "/"));

    // an old settings.json has no budgets: the default is empty, and a bad value changes nothing
    assert_eq!(app.call("settings.get", json!({})).unwrap()["budgets"], json!({}));
    for bad in [json!({"default": {"daily": 0}}), json!({"default": {"auto_pause": "yes"}}), json!({"nope": 1}), json!("x")] {
        assert!(app.call("settings.set", json!({"budgets": bad})).is_err());
    }
    assert_eq!(app.call("settings.get", json!({})).unwrap()["budgets"], json!({}));
    // no limits: nothing to report
    assert_eq!(app.call("budget.check", json!({})).unwrap()["budgets"], json!([]));

    // `a` has a limit and no opt-in; `b` has the opt-in
    let budgets = json!({"worktrees": {a.clone(): {"daily": 1000}, b.clone(): {"daily": 1000, "auto_pause": true}}});
    app.call("settings.set", json!({"budgets": budgets})).unwrap();
    let got = app.call("settings.get", json!({})).unwrap()["budgets"].clone();
    assert_eq!(got["worktrees"][&a], json!({"daily": 1000}), "auto_pause stays off unless asked for");
    assert_eq!(got["worktrees"][&b]["auto_pause"], json!(true));
    let (sa, sb) = (spawn_working_agent(&mut app, &a), spawn_working_agent(&mut app, &b));

    // first look: the day starts here, nothing is over
    let rows = app.call("budget.check", json!({})).unwrap()["budgets"].clone();
    assert_eq!(rows.as_array().unwrap().len(), 2);
    assert!(events(&mut app).is_empty());

    // 85 %: a warning for each, nobody is interrupted
    log_tokens(&home, &a, "m1", 850);
    log_tokens(&home, &b, "m1", 850);
    app.call("budget.check", json!({})).unwrap();
    let ev = events(&mut app);
    assert_eq!(ev.len(), 2, "{ev:?}");
    assert!(ev.iter().all(|e| e["level"] == "warn" && e["period"] == "daily" && e["pct"] == 85 && e["paused"] == json!([])), "{ev:?}");
    app.call("budget.check", json!({})).unwrap();
    assert!(events(&mut app).is_empty(), "the same level is not reported again");

    // over: both are reported; only the one with the opt-in has its agent interrupted
    log_tokens(&home, &a, "m2", 650);
    log_tokens(&home, &b, "m2", 650);
    let status = app.call("budget.check", json!({})).unwrap()["budgets"].clone();
    assert!(status.as_array().unwrap().iter().all(|r| r["level"] == "over" && r["used"] == 1500 && r["limit"] == 1000));
    let ev = events(&mut app);
    assert_eq!(ev.len(), 2, "{ev:?}");
    for e in &ev {
        let want = if e["worktree"] == json!(b) { json!([sb]) } else { json!([]) };
        assert_eq!(e["level"], "over");
        assert_eq!(e["paused"], want, "{e}");
    }
    assert_ne!(sa, sb);
    app.call("budget.check", json!({})).unwrap();
    assert!(events(&mut app).is_empty(), "over is reported once per period");

    // read only status for given paths, nothing recorded
    let s = app.call("budget.status", json!({"worktrees": [a.clone()]})).unwrap();
    assert_eq!(s["budgets"][0]["pct"], 150);
    assert!(events(&mut app).is_empty());

    // the state survives: a new daemon on the same folder does not report the same crossing again
    drop(app);
    let (pipe2, token2) = start(&root.join("d"), "again");
    let mut again = Client::connect(&pipe2, &token2).unwrap();
    again.call("budget.check", json!({})).unwrap();
    assert!(events(&mut again).is_empty());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn an_agent_cannot_change_or_take_the_budget_events() {
    let root = std::env::temp_dir().join(format!("axelstra-budget-role-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let (pipe, token) = start(&root, "role");
    let mut app = Client::connect(&pipe, &token).unwrap();
    let (prog, args) = if cfg!(windows) {
        ("cmd", json!(["/c", "echo TOK[%AXELSTRA_AGENT_TOKEN%] & ping -n 20 127.0.0.1 >nul"]))
    } else {
        ("sh", json!(["-c", "echo TOK[$AXELSTRA_AGENT_TOKEN]; sleep 20"]))
    };
    let id = app.call("session.spawn", json!({"program": prog, "args": args})).unwrap()["id"].as_u64().unwrap();
    let t = Instant::now();
    let agent_token = loop {
        let lines = app.call("session.screen", json!({"id": id})).unwrap()["lines"].to_string();
        if let Some(i) = lines.find("TOK[") {
            let rest = &lines[i + 4..];
            if let Some(j) = rest.find(']').filter(|j| *j >= 32) {
                break rest[..j].to_string();
            }
        }
        assert!(t.elapsed() < Duration::from_secs(15), "no token printed: {lines}");
        std::thread::sleep(Duration::from_millis(200));
    };
    let mut agent = Client::connect_agent(&pipe, &"x".repeat(32), &agent_token).unwrap();
    for (m, p) in [
        ("settings.set", json!({"budgets": {"default": {"weekly": 999_999_999}}})),
        ("settings.set", json!({"budgets": null})),
        ("budget.events", json!({})),
        ("budget.check", json!({})),
    ] {
        let e = agent.call(m, p).unwrap_err().to_string();
        assert!(e.contains("not allowed to agents"), "{m}: {e}");
    }
    // the rest of settings and the read-only status stay available to it
    assert!(agent.call("settings.set", json!({"notify_sound": false})).is_ok());
    assert!(agent.call("budget.status", json!({"worktrees": []})).is_ok());
    assert_eq!(app.call("settings.get", json!({})).unwrap()["budgets"], json!({}));
    let _ = std::fs::remove_dir_all(&root);
}
