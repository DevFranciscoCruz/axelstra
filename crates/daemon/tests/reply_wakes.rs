//! A quiet reply (what `axelstra reply` sends to a blocked `axelstra ask`) must wake the watch stream by itself:
//! the asker's `ask` loop and `axelstra wait` sleep until `session.changed`, and nothing else need happen at that moment.
mod common;
use axelstra_daemon::client::{Client, Frame};
use axelstra_daemon::server;
use serde_json::{json, Value};
use std::sync::mpsc;
use std::time::Duration;

fn start(tag: &str) -> (String, String) {
    let root = std::env::temp_dir().join(format!("axelstra-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
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

fn spawn(c: &mut Client, label: &str) -> u64 {
    let (prog, args) = if cfg!(windows) { ("cmd", json!(["/q"])) } else { ("sh", json!([])) };
    c.call("session.spawn", json!({"program": prog, "args": args, "label": label, "meta": {"kind": "agent"}})).unwrap()["id"]
        .as_u64()
        .unwrap()
}

#[test]
fn a_quiet_reply_wakes_the_watch_stream_on_its_own() {
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(90));
        eprintln!("watchdog: test stuck");
        std::process::exit(1);
    });
    let (pipe, token) = start("replywake");
    let mut c = Client::connect(&pipe, &token).unwrap();
    let asker = spawn(&mut c, "asker");
    let boss = spawn(&mut c, "boss");
    let q = c.call("msg.send", json!({"from": asker, "to": boss, "text": "which db?", "kind": "ask", "quiet": true})).unwrap()["id"]
        .as_u64()
        .unwrap();

    let mut w = Client::connect(&pipe, &token).unwrap();
    w.call("watch.unsubscribe", json!({})).unwrap();
    let (tx, rx) = mpsc::channel::<Value>();
    std::thread::spawn(move || {
        while let Ok(f) = w.read() {
            if let Frame::Event(e) = f {
                let _ = tx.send(e);
            }
        }
    });
    // the asker has one unread message, then reads it (a read bumps nothing): the reply will put its unread count back
    // to the value the session monitor last saw
    c.call("msg.send", json!({"from": boss, "to": asker, "text": "fyi", "kind": "message", "quiet": true})).unwrap();
    // let the shell banners settle and the monitor go back to sleep (no working session left, so no timer either)
    std::thread::sleep(Duration::from_millis(3500));
    c.call("msg.inbox", json!({"session": asker})).unwrap();
    while rx.try_recv().is_ok() {}
    std::thread::sleep(Duration::from_millis(300));
    while rx.try_recv().is_ok() {}

    // the reply lands: unread is 1 again, exactly what the monitor last saw, so only the message itself can announce
    c.call("msg.send", json!({"from": boss, "to": asker, "text": "postgres", "kind": "reply", "reply_to": q, "quiet": true})).unwrap();
    let ev = rx.recv_timeout(Duration::from_secs(10)).expect("a reply must announce itself with session.changed");
    assert_eq!(ev["event"], "session.changed");
    assert_eq!(c.call("msg.reply_of", json!({"id": q})).unwrap()["reply"]["text"], "postgres");
}
