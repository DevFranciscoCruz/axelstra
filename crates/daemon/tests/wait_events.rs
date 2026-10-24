//! "Waiting for an answer" comes from events only: `axelstra ask`/`escalate` and an agent's own hook (`session.hint`
//! with a kind and text). The screen is never read: prompts, questions and finished-turn lines on it change nothing.
mod common;
use axelstra_daemon::client::Client;
use axelstra_daemon::server;
use serde_json::{json, Value};
use std::time::{Duration, Instant};

fn shell() -> (String, Vec<String>) {
    if cfg!(windows) {
        ("cmd".into(), vec!["/q".into()])
    } else {
        ("sh".into(), vec![])
    }
}

fn spawn(c: &mut Client, label: &str, extra: Value) -> u64 {
    let (prog, args) = shell();
    let mut p = json!({"program": prog, "args": args, "label": label, "meta": {"kind": "agent"}});
    for (k, v) in extra.as_object().unwrap() {
        p[k] = v.clone();
    }
    c.call("session.spawn", p).unwrap()["id"].as_u64().unwrap()
}

fn row(c: &mut Client, id: u64) -> Value {
    c.call("session.list", json!({})).unwrap().as_array().unwrap().iter().find(|s| s["id"] == id).unwrap().clone()
}

fn start(tag: &str) -> (String, String) {
    let root = std::env::temp_dir().join(format!("axelstra-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let pipe = format!("{}-{tag}-{}", common::base_pipe(), std::process::id());
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
    (pipe, token)
}

#[test]
fn only_events_make_a_session_wait_and_the_parent_hears_once() {
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(90));
        eprintln!("watchdog: test stuck");
        std::process::exit(1);
    });
    let (pipe, token) = start("waitev");
    let mut c = Client::connect(&pipe, &token).unwrap();
    let boss = spawn(&mut c, "boss", json!({}));
    let w = spawn(&mut c, "worker", json!({"parent": boss}));
    // let the shell banner settle: a hint counts as a wait while nothing was printed after it
    std::thread::sleep(Duration::from_millis(2500));

    // 1. no event: even a screen full of prompts, questions and turn summaries is not a wait
    let (prog, args) = if cfg!(windows) {
        ("cmd", json!(["/c", "echo Do you want to proceed? (y/n) & echo Posso continuar? & echo Cooked for 2m 18s - done & ping -n 30 127.0.0.1 >nul"]))
    } else {
        ("sh", json!(["-c", "echo 'Do you want to proceed? (y/n)'; echo 'Posso continuar?'; echo 'Cooked for 2m 18s - done'; sleep 30"]))
    };
    let screeny = c.call("session.spawn", json!({"program": prog, "args": args, "label": "screeny", "meta": {"kind": "agent"}, "parent": boss})).unwrap()["id"]
        .as_u64()
        .unwrap();
    c.call("session.subscribe", json!({"id": w})).unwrap();
    // the user's own text with a question mark
    c.send_input(w as u32, b"> posso avancar com isto?\r").unwrap();
    std::thread::sleep(Duration::from_millis(3000));
    assert_eq!(row(&mut c, screeny)["waiting"], false, "screen text must never make a session wait");
    assert_eq!(row(&mut c, w)["waiting"], false);
    assert!(row(&mut c, w)["wait"].is_null());

    // 2. a permission event: waiting, with the agent's text, and the parent is told
    std::thread::sleep(Duration::from_millis(2500));
    c.call("session.hint", json!({"id": w, "state": "waiting", "kind": "permission", "text": "Claude needs your permission to use Bash"})).unwrap();
    let r = row(&mut c, w);
    assert_eq!(r["waiting"], true);
    assert_eq!(r["wait"], json!({"kind": "permission", "text": "Claude needs your permission to use Bash"}));
    let told = c.call("msg.inbox", json!({"session": boss})).unwrap();
    assert_eq!(told.as_array().unwrap().len(), 1, "{told}");
    assert!(told[0]["text"].as_str().unwrap().contains("needs your permission to use Bash"));
    // the same event again does not tell the parent again
    c.call("session.hint", json!({"id": w, "state": "waiting", "kind": "permission", "text": "Claude needs your permission to use Bash"})).unwrap();
    assert_eq!(c.call("msg.inbox", json!({"session": boss})).unwrap().as_array().unwrap().len(), 0);

    // 3. Stop / UserPromptSubmit clear it
    c.call("session.hint", json!({"id": w, "state": "idle"})).unwrap();
    assert_eq!(row(&mut c, w)["waiting"], false);
    c.call("session.hint", json!({"id": w, "state": "waiting", "kind": "question", "text": "Which database?"})).unwrap();
    assert_eq!(row(&mut c, w)["wait"]["kind"], "question");
    c.call("session.hint", json!({"id": w, "state": "working"})).unwrap();
    assert_eq!(row(&mut c, w)["waiting"], false);

    // 4. idle_input (Claude finished and sits at its prompt) and a "waiting" with no kind are not waits
    c.call("session.hint", json!({"id": w, "state": "waiting", "kind": "idle_input", "text": "Claude is waiting for your input"})).unwrap();
    assert_eq!(row(&mut c, w)["waiting"], false);
    c.call("session.hint", json!({"id": w, "state": "waiting"})).unwrap();
    assert_eq!(row(&mut c, w)["waiting"], false);

    // 5. ask is a wait with the question until the reply; the reply clears it
    let q = c.call("msg.send", json!({"from": w, "to": boss, "text": "Postgres or SQLite?", "kind": "ask", "quiet": true})).unwrap()["id"].as_u64().unwrap();
    let r = row(&mut c, w);
    assert_eq!((r["waiting"].clone(), r["wait"]["text"].clone()), (json!(true), json!("Postgres or SQLite?")));
    c.call("msg.send", json!({"from": boss, "to": w, "text": "SQLite", "kind": "reply", "reply_to": q, "quiet": true})).unwrap();
    assert_eq!(row(&mut c, w)["waiting"], false);

    // 6. a session that reported with done never waits by hint
    c.call("session.hint", json!({"id": w, "state": "waiting", "kind": "permission", "text": "p"})).unwrap();
    c.call("msg.send", json!({"from": w, "to": boss, "text": "[ok] done", "kind": "done", "quiet": true})).unwrap();
    assert_eq!(row(&mut c, w)["waiting"], false);

    // 7. a bad state is refused
    assert!(c.call("session.hint", json!({"id": w, "state": "screen"})).is_err());
}

#[test]
fn an_agent_can_only_say_that_it_itself_waits() {
    let (pipe, token) = start("waitrole");
    let mut app = Client::connect(&pipe, &token).unwrap();
    let (prog, args) = if cfg!(windows) {
        ("cmd", json!(["/c", "echo TOK[%AXELSTRA_AGENT_TOKEN%] & ping -n 20 127.0.0.1 >nul"]))
    } else {
        ("sh", json!(["-c", "echo TOK[$AXELSTRA_AGENT_TOKEN]; sleep 20"]))
    };
    let me = app.call("session.spawn", json!({"program": prog, "args": args, "meta": {"kind": "agent"}})).unwrap()["id"].as_u64().unwrap();
    let other = spawn(&mut app, "other", json!({}));
    let t = Instant::now();
    let agent_token = loop {
        let lines = app.call("session.screen", json!({"id": me})).unwrap()["lines"].to_string();
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
    let mut agent = Client::connect_agent(&pipe, &"x".repeat(32), &agent_token).unwrap();
    let e = agent.call("session.hint", json!({"id": other, "state": "waiting", "kind": "permission", "text": "x"})).unwrap_err().to_string();
    assert!(e.contains("own session"), "{e}");
    assert!(agent.call("session.hint", json!({"id": me, "state": "waiting", "kind": "permission", "text": "mine"})).is_ok());
    assert_eq!(row(&mut app, me)["wait"]["text"], "mine");
    assert_eq!(row(&mut app, other)["waiting"], false);
}
