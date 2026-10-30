//! `msg.wait` (`axelstra inbox --wait`): a coordinator blocks until a report or a pendency reaches it, with no polling
//! and no model turns; and the pull fields of `session.list` it reads instead of being told about state.
mod common;
use axelstra_daemon::client::Client;
use axelstra_daemon::server;
use serde_json::{json, Value};
use std::path::Path;
use std::time::{Duration, Instant};

fn serve(root: &Path, tag: &str, token: &str) -> String {
    let pipe = format!("{}-mwait-{tag}-{}", common::base_pipe(), std::process::id());
    let (p, t, s) = (pipe.clone(), token.to_string(), root.join("projects.json"));
    std::thread::spawn(move || {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap()
            .block_on(server::serve_with_store(p, t, s))
            .unwrap();
    });
    common::wait_ready(&pipe, token);
    pipe
}

fn temp(tag: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!("axelstra-mwait-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    root
}

/// A long-lived shell that prints its own agent credential: its id and a client that uses it.
fn agent_session(app: &mut Client, pipe: &str, extra: Value) -> (u32, Client) {
    let (prog, args) = if cfg!(windows) {
        ("cmd", json!(["/q", "/k", "echo TOK[%AXELSTRA_AGENT_TOKEN%]"]))
    } else {
        ("sh", json!(["-c", "echo TOK[$AXELSTRA_AGENT_TOKEN]; exec cat"]))
    };
    let mut params = json!({"program": prog, "args": args, "meta": {"kind": "agent"}});
    for (k, v) in extra.as_object().cloned().unwrap_or_default() {
        params[k] = v;
    }
    let id = app.call("session.spawn", params).unwrap()["id"].as_u64().unwrap() as u32;
    let t = Instant::now();
    let token = loop {
        let lines = app.call("session.screen", json!({"id": id, "lines": 100})).unwrap()["lines"].to_string();
        if let Some(a) = lines.find("TOK[") {
            let rest = &lines[a + 4..];
            if let Some(b) = rest.find(']') {
                if b >= 32 {
                    break rest[..b].to_string();
                }
            }
        }
        assert!(t.elapsed() < Duration::from_secs(15), "no token printed: {lines}");
        std::thread::sleep(Duration::from_millis(200));
    };
    (id, Client::connect_agent(pipe, &"x".repeat(32), &token).unwrap())
}

fn texts(r: &Value) -> Vec<String> {
    r["messages"].as_array().unwrap().iter().map(|m| m["text"].as_str().unwrap().to_string()).collect()
}

fn row(app: &mut Client, id: u32) -> Value {
    app.call("session.list", json!({})).unwrap().as_array().unwrap().iter().find(|s| s["id"] == id).unwrap().clone()
}

#[test]
fn msg_wait_returns_blocks_times_out_filters_and_stays_on_its_own_session() {
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(120));
        eprintln!("watchdog: test stuck");
        std::process::exit(1);
    });
    let root = temp("main");
    let token = "m".repeat(32);
    let pipe = serve(&root, "main", &token);
    let mut app = Client::connect(&pipe, &token).unwrap();
    let (boss, mut boss_c) = agent_session(&mut app, &pipe, json!({}));
    let (kid, mut kid_c) = agent_session(&mut app, &pipe, json!({"parent": boss}));

    // the pull fields are on every row
    let r = row(&mut app, kid);
    assert_eq!(r["liveness"]["pty"], true);
    assert!(r["next_action"].is_string(), "{r}");
    assert!(r.get("quiet_since").is_some(), "{r}");

    // nothing pending: an empty answer once the limit runs out, flagged timed_out
    let t = Instant::now();
    let r = boss_c.call("msg.wait", json!({"session": boss, "timeout_ms": 300})).unwrap();
    assert_eq!(r["timed_out"], true);
    assert!(texts(&r).is_empty());
    assert!(t.elapsed() >= Duration::from_millis(250), "it waited");

    // a pending done is handed back at once, marked read
    kid_c.call("msg.send", json!({"to": boss, "text": "[ok] first", "kind": "done", "quiet": true})).unwrap();
    let t = Instant::now();
    let r = boss_c.call("msg.wait", json!({"session": boss, "types": ["done"], "timeout_ms": 10_000})).unwrap();
    assert_eq!(texts(&r), vec!["[ok] first"]);
    assert_eq!(r["timed_out"], false);
    assert!(t.elapsed() < Duration::from_secs(5), "at once");
    assert_eq!(boss_c.call("msg.inbox", json!({"session": boss})).unwrap().as_array().unwrap().len(), 0, "read");

    // types filter: a plain message does not end a wait for done, and stays unread
    kid_c.call("msg.send", json!({"to": boss, "text": "fyi", "kind": "message", "quiet": true})).unwrap();
    let r = boss_c.call("msg.wait", json!({"session": boss, "types": ["done", "question"], "timeout_ms": 300})).unwrap();
    assert_eq!(r["timed_out"], true);
    // a question is a pendency; --no-mark leaves it unread
    kid_c.call("msg.send", json!({"to": boss, "text": "which db?", "kind": "ask", "quiet": true})).unwrap();
    let r = boss_c.call("msg.wait", json!({"session": boss, "types": ["question"], "mark": false, "timeout_ms": 300})).unwrap();
    assert_eq!(texts(&r), vec!["which db?"]);
    let q = r["messages"][0]["id"].as_u64().unwrap();
    assert_eq!(row(&mut app, kid)["next_action"], format!("reply {q}"));
    let unread: Vec<String> = boss_c
        .call("msg.inbox", json!({"session": boss}))
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["text"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(unread, vec!["fyi", "which db?"]);
    boss_c.call("msg.send", json!({"to": kid, "text": "postgres", "kind": "reply", "reply_to": q, "quiet": true})).unwrap();

    // blocks, then returns when a done arrives
    let waiter = {
        let (p, b) = (pipe.clone(), boss);
        let mut c = Client::connect(&p, &token).unwrap();
        std::thread::spawn(move || {
            let t = Instant::now();
            let r = c.call("msg.wait", json!({"session": b, "types": ["done"], "timeout_ms": 15_000})).unwrap();
            (r, t.elapsed())
        })
    };
    std::thread::sleep(Duration::from_millis(500));
    kid_c.call("msg.send", json!({"to": boss, "text": "[ok] second", "kind": "done", "quiet": true})).unwrap();
    let (r, took) = waiter.join().unwrap();
    assert_eq!(texts(&r), vec!["[ok] second"]);
    assert!(took >= Duration::from_millis(400) && took < Duration::from_secs(10), "{took:?}");

    // an agent waits only on its own session
    let e = kid_c.call("msg.wait", json!({"session": boss, "timeout_ms": 100})).unwrap_err();
    assert!(format!("{e:#}").contains("own session"), "{e:#}");
    // an unknown type is refused
    let e = boss_c.call("msg.wait", json!({"session": boss, "types": ["bogus"], "timeout_ms": 100})).unwrap_err();
    assert!(format!("{e:#}").contains("bogus"), "{e:#}");
}
