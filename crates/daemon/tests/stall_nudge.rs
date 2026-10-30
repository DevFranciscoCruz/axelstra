//! The stop reminder: an Axelstra-managed agent whose own Stop hook (`session.hint` with `event: "stop"`) says its turn
//! ended with nothing that explains it gets ONE short line typed into it; the parent hears only when it stays quiet a
//! threshold after it.
mod common;
use axelstra_daemon::client::Client;
use axelstra_daemon::server;
use serde_json::{json, Value};
use std::time::{Duration, Instant};

const NUDGE: &str = "Yourturnendedwithoutdone,awaitorask";

fn shell() -> (String, Vec<String>) {
    if cfg!(windows) {
        ("cmd".into(), vec!["/q".into()])
    } else {
        // `cat` only echoes: a shell would run the backticks of the reminder
        ("cat".into(), vec![])
    }
}

fn spawn(c: &mut Client, label: &str, meta: Value, extra: Value) -> u64 {
    let (prog, args) = shell();
    let mut p = json!({"program": prog, "args": args, "label": label, "meta": meta, "goal": "do the task"});
    for (k, v) in extra.as_object().unwrap() {
        p[k] = v.clone();
    }
    c.call("session.spawn", p).unwrap()["id"].as_u64().unwrap()
}

/// How many times the reminder shows on the session's screen (lines joined and spaces dropped: a wrapped line still counts).
fn nudges(c: &mut Client, id: u64) -> usize {
    let r = c.call("session.screen", json!({"id": id, "lines": 200})).unwrap();
    let all: String = r["lines"].as_array().unwrap().iter().filter_map(Value::as_str).collect::<String>();
    all.chars().filter(|c| !c.is_whitespace()).collect::<String>().matches(NUDGE).count()
}

fn stop(c: &mut Client, id: u64) {
    c.call("session.hint", json!({"id": id, "state": "idle", "event": "stop"})).unwrap();
}

fn told(c: &mut Client, boss: u64, id: u64) -> Vec<String> {
    let inbox = c.call("msg.inbox", json!({"session": boss, "all": true})).unwrap();
    let me = format!("(session {id})");
    inbox.as_array().unwrap().iter().filter_map(|m| m["text"].as_str().map(String::from)).filter(|t| t.contains(&me)).collect()
}

#[test]
fn the_stop_hook_reminds_once_and_only_an_unexplained_stop() {
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(90));
        eprintln!("watchdog: test stuck");
        std::process::exit(1);
    });
    common::data_dir();
    std::env::set_var("AXELSTRA_STALL_SECS", "3");
    let root = std::env::temp_dir().join(format!("axelstra-nudge-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let pipe = format!("{}-nudge-{}", common::base_pipe(), std::process::id());
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

    let agent = json!({"kind": "agent"});
    let boss = spawn(&mut c, "boss", agent.clone(), json!({}));
    let worker = spawn(&mut c, "worker", agent.clone(), json!({"parent": boss}));
    let awaiting = spawn(&mut c, "awaiting", agent.clone(), json!({"parent": boss}));
    let done = spawn(&mut c, "done", agent.clone(), json!({"parent": boss}));
    let asking = spawn(&mut c, "asking", agent.clone(), json!({"parent": boss}));
    let lead = spawn(&mut c, "lead", agent.clone(), json!({"parent": boss}));
    let _grandchild = spawn(&mut c, "grandchild", agent.clone(), json!({"parent": lead}));
    let driven = spawn(&mut c, "driven", agent.clone(), json!({"parent": boss}));
    let lonely = spawn(&mut c, "lonely", agent.clone(), json!({}));
    // under another parent, so its own stall line does not join the worker's
    let old_cli = spawn(&mut c, "old", agent.clone(), json!({"parent": lead}));
    c.call("session.await", json!({"id": awaiting, "reason": "CI #1"})).unwrap();
    c.call("msg.send", json!({"from": done, "to": boss, "text": "[ok] done", "kind": "done", "quiet": true})).unwrap();
    c.call("msg.send", json!({"from": asking, "to": boss, "text": "A or B?", "kind": "ask", "quiet": true})).unwrap();
    c.send_input(driven as u32, b"x").unwrap();

    // the managed worker's turn ended with nothing that explains it: one reminder
    stop(&mut c, worker);
    let t0 = Instant::now();
    while nudges(&mut c, worker) == 0 {
        assert!(t0.elapsed() < Duration::from_secs(10), "never reminded");
        std::thread::sleep(Duration::from_millis(100));
    }
    std::thread::sleep(Duration::from_millis(800));
    let once = nudges(&mut c, worker);
    // the reminder's own turn (a working hint) and the stop that ends it: no second reminder
    c.call("session.hint", json!({"id": worker, "state": "working"})).unwrap();
    stop(&mut c, worker);
    stop(&mut c, worker);
    std::thread::sleep(Duration::from_millis(800));
    assert_eq!(nudges(&mut c, worker), once, "reminded twice in one stop episode");

    // nothing for an explained stop, a user-driven one, one with no parent agent, or an old hook with no event
    for id in [awaiting, done, asking, lead, driven, lonely] {
        stop(&mut c, id);
    }
    c.call("session.hint", json!({"id": old_cli, "state": "idle"})).unwrap();
    std::thread::sleep(Duration::from_millis(1500));
    for (id, why) in [
        (awaiting, "awaiting"),
        (done, "done"),
        (asking, "ask"),
        (lead, "live child"),
        (driven, "user driven"),
        (lonely, "no parent"),
        (old_cli, "old hook"),
    ] {
        assert_eq!(nudges(&mut c, id), 0, "{why}");
    }

    // it stays quiet a threshold after the reminder: the parent hears it ONCE, in one line
    let t0 = Instant::now();
    let got = loop {
        let got = told(&mut c, boss, worker);
        if !got.is_empty() {
            break got;
        }
        assert!(t0.elapsed() < Duration::from_secs(15), "parent never told");
        std::thread::sleep(Duration::from_millis(250));
    };
    assert!(got[0].contains("ignored one reminder") && got[0].contains("axelstra read"), "{}", got[0]);
    std::thread::sleep(Duration::from_secs(4));
    assert_eq!(told(&mut c, boss, worker).len(), 1, "told twice");

    // new work (a message) and another stop: a new episode, but the 10 min rate limit holds the reminder back
    c.call("msg.send", json!({"to": worker, "text": "go on", "quiet": true})).unwrap();
    std::thread::sleep(Duration::from_millis(50));
    stop(&mut c, worker);
    std::thread::sleep(Duration::from_millis(800));
    assert_eq!(nudges(&mut c, worker), once, "rate limited");
    let _ = std::fs::remove_dir_all(&root);
}
