use axelstra_daemon::client::{Client, Frame};
use axelstra_daemon::server;
use serde_json::{json, Value};
use std::time::{Duration, Instant};

fn shell() -> (String, Vec<String>) {
    if cfg!(windows) { ("cmd".into(), vec!["/q".into()]) } else { ("sh".into(), vec![]) }
}

fn spawn(c: &mut Client, label: &str, extra: Value) -> u64 {
    let (prog, args) = shell();
    let mut p = json!({"program": prog, "args": args, "label": label, "meta": {"kind": "agent"}});
    for (k, v) in extra.as_object().unwrap() {
        p[k] = v.clone();
    }
    c.call("session.spawn", p).unwrap()["id"].as_u64().unwrap()
}

fn list(c: &mut Client) -> Vec<Value> {
    c.call("session.list", json!({})).unwrap().as_array().unwrap().clone()
}

#[test]
fn messages_ask_done_escalate_and_the_worker_limit() {
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(60));
        eprintln!("watchdog: test stuck");
        std::process::exit(1);
    });
    let root = std::env::temp_dir().join(format!("axelstra-bus-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let pipe = format!("{}-bus-{}", axelstra_daemon::proto::pipe_name(), std::process::id());
    let token = "c".repeat(32);
    let (p, t, s) = (pipe.clone(), token.clone(), root.join("projects.json"));
    std::thread::spawn(move || {
        tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap()
            .block_on(server::serve_with_store(p, t, s)).unwrap();
    });
    std::thread::sleep(Duration::from_millis(300));
    let mut c = Client::connect(&pipe, &token).unwrap();

    let boss = spawn(&mut c, "boss", json!({}));
    let helper = spawn(&mut c, "helper", json!({"parent": boss}));

    // parent is recorded in the session's meta
    let l = list(&mut c);
    assert_eq!(l.iter().find(|s| s["id"] == helper).unwrap()["meta"]["parent"], boss);

    // a plain message lands in the inbox, once
    let m = c.call("msg.send", json!({"from": boss, "to": helper, "text": "hello", "quiet": true})).unwrap()["id"].as_u64().unwrap();
    assert_eq!(list(&mut c).iter().find(|s| s["id"] == helper).unwrap()["unread"], 1);
    let inbox = c.call("msg.inbox", json!({"session": helper})).unwrap();
    assert_eq!(inbox[0]["text"], "hello");
    assert_eq!(c.call("msg.inbox", json!({"session": helper})).unwrap().as_array().unwrap().len(), 0);
    let _ = m;

    // ask / reply
    let q = c.call("msg.send", json!({"from": helper, "to": boss, "text": "which db?", "kind": "ask", "quiet": true})).unwrap()["id"].as_u64().unwrap();
    assert!(c.call("msg.reply_of", json!({"id": q})).unwrap()["reply"].is_null());
    c.call("msg.send", json!({"from": boss, "to": helper, "text": "postgres", "kind": "reply", "reply_to": q, "quiet": true})).unwrap();
    assert_eq!(c.call("msg.reply_of", json!({"id": q})).unwrap()["reply"]["text"], "postgres");

    // escalate shows as waiting until the parent answers
    c.call("msg.send", json!({"from": helper, "to": boss, "text": "stuck", "kind": "escalate", "quiet": true})).unwrap();
    let h = list(&mut c).into_iter().find(|s| s["id"] == helper).unwrap();
    assert_eq!(h["waiting"], true);
    assert_eq!(h["escalated"], true);
    c.call("msg.send", json!({"from": boss, "to": helper, "text": "try X", "quiet": true})).unwrap();
    assert_eq!(list(&mut c).into_iter().find(|s| s["id"] == helper).unwrap()["escalated"], false);

    // the worker limit counts live workers only
    assert_eq!(c.call("orch.limits", json!({"max_workers": 1})).unwrap()["max_workers"], 1);
    let w1 = spawn(&mut c, "w1", json!({"worker": true, "parent": boss}));
    let (prog, args) = shell();
    let err = c.call("session.spawn", json!({"program": prog, "args": args, "worker": true, "parent": boss})).unwrap_err();
    assert!(err.to_string().contains("worker limit"), "{err}");
    c.call("session.kill", json!({"id": w1})).unwrap();
    let t0 = Instant::now();
    loop {
        let (prog, args) = shell();
        if c.call("session.spawn", json!({"program": prog, "args": args, "worker": true, "parent": boss})).is_ok() {
            break;
        }
        assert!(t0.elapsed() < Duration::from_secs(10), "limit never freed");
        std::thread::sleep(Duration::from_millis(200));
    }
    c.call("orch.limits", json!({"max_workers": 6})).unwrap();

    // a reaped worker that calls `done` is closed and its report reaches the parent's screen
    let worker = spawn(&mut c, "reaped", json!({"reap": {"parent": boss, "mode": "idle", "idle_secs": 600}}));
    c.call("msg.send", json!({"from": worker, "to": boss, "text": "[ok] report-xyz-77", "kind": "done"})).unwrap();
    let t0 = Instant::now();
    while list(&mut c).iter().any(|s| s["id"] == worker) {
        assert!(t0.elapsed() < Duration::from_secs(15), "worker never closed");
        std::thread::sleep(Duration::from_millis(200));
    }
    c.call("session.subscribe", json!({"id": boss})).unwrap();
    let t0 = Instant::now();
    let mut seen = String::new();
    while !seen.contains("report-xyz-77") {
        assert!(t0.elapsed() < Duration::from_secs(10), "boss never got the report: {seen}");
        if let Ok(Frame::Output { session, data }) = c.read() {
            if session as u64 == boss {
                seen.push_str(&String::from_utf8_lossy(&data));
            }
        }
    }
    let _ = std::fs::remove_dir_all(&root);
}
