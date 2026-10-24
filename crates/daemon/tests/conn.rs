mod common;
use axelstra_daemon::client::{Client, Frame};
use axelstra_daemon::proto::{BUILD_ID, PROTO, SECURITY_MIN};
use axelstra_daemon::server;
use serde_json::json;
use std::time::{Duration, Instant};

fn start() -> (String, String) {
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(90));
        eprintln!("watchdog: test stuck");
        std::process::exit(1);
    });
    let pipe = format!("{}-conn-{}-{:?}", common::base_pipe(), std::process::id(), std::thread::current().id())
        .replace("ThreadId(", "")
        .replace(')', "");
    let token = "c".repeat(32);
    let (p, t) = (pipe.clone(), token.clone());
    std::thread::spawn(move || {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap()
            .block_on(server::serve_with_store(p, t, common::store("srv")))
            .unwrap();
    });
    std::thread::sleep(Duration::from_millis(300));
    (pipe, token)
}

#[test]
fn hello_and_ping_report_the_build() {
    let (pipe, token) = start();
    let mut c = Client::connect(&pipe, &token).unwrap();
    assert_eq!(c.build, BUILD_ID);
    assert_eq!(c.security, SECURITY_MIN);
    assert_eq!(c.proto, PROTO);
    let ping = c.call("ping", json!({})).unwrap();
    assert_eq!(ping["build"], BUILD_ID);
    assert_eq!(ping["security"], SECURITY_MIN);
}

#[test]
fn bad_tokens_are_slowed_down() {
    let (pipe, _) = start();
    let t0 = Instant::now();
    for _ in 0..4 {
        assert!(Client::connect(&pipe, "nope").is_err());
    }
    // 50 + 100 + 150 + 200 ms of throttle
    assert!(t0.elapsed() >= Duration::from_millis(450), "{:?}", t0.elapsed());
}

#[test]
fn pipelined_calls_are_answered_in_order() {
    let (pipe, token) = start();
    let mut c = Client::connect(&pipe, &token).unwrap();
    for _ in 0..20 {
        c.send("ping", json!({})).unwrap();
    }
    let mut ids = vec![];
    while ids.len() < 20 {
        if let Frame::Response(v) = c.read().unwrap() {
            ids.push(v["id"].as_u64().unwrap());
        }
    }
    let mut sorted = ids.clone();
    sorted.sort();
    assert_eq!(ids, sorted);
}

#[test]
fn keys_still_flow_while_a_slow_call_runs() {
    let (pipe, token) = start();
    let mut c = Client::connect(&pipe, &token).unwrap();
    let (prog, args): (&str, Vec<&str>) = if cfg!(windows) { ("cmd", vec!["/q"]) } else { ("sh", vec![]) };
    let id =
        c.call("session.spawn", json!({"program": prog, "args": args, "cols": 80, "rows": 24})).unwrap()["id"].as_u64().unwrap() as u32;
    c.call("session.subscribe", json!({"id": id})).unwrap();
    // a call that takes a while (a scan of the whole disk would): queued, not run inline on the reader
    c.send("session.list", json!({})).unwrap();
    let eol = if cfg!(windows) { "\r\n" } else { "\n" };
    c.send_input(id, format!("echo still-typing{eol}").as_bytes()).unwrap();
    let mut all = Vec::new();
    let end = Instant::now() + Duration::from_secs(10);
    while Instant::now() < end {
        if let Ok(Frame::Output { session, data }) = c.read() {
            if session == id {
                all.extend(data);
                if String::from_utf8_lossy(&all).contains("still-typing") {
                    return;
                }
            }
        }
    }
    panic!("keystrokes were not delivered");
}
