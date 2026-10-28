//! A terminal stream ends when its client goes away, with no heartbeat frame: the forwarder is woken by the closed
//! connection, even if the session never prints again.
mod common;
use axelstra_daemon::client::Client;
use axelstra_daemon::server;
use serde_json::json;
use std::time::{Duration, Instant};

fn until(what: &str, mut ok: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(20);
    while !ok() {
        assert!(Instant::now() < end, "timed out: {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn a_dead_client_ends_a_quiet_stream_without_a_heartbeat() {
    let pipe = common::pipe("streamclose");
    let token = "s".repeat(32);
    let (p, t, s) = (pipe.clone(), token.clone(), common::store("streamclose"));
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
    let (prog, args) = if cfg!(windows) { ("cmd", json!(["/q"])) } else { ("sh", json!([])) };
    let id = c.call("session.spawn", json!({"program": prog, "args": args})).unwrap()["id"].as_u64().unwrap();
    let before = server::live_streams();
    c.call("session.subscribe", json!({"id": id})).unwrap();
    until("stream registered", || server::live_streams() == before + 1);

    // the session stays silent from here on; only the client going away can end the stream
    drop(c);
    until("stream ended after the client left", || server::live_streams() == before);

    let mut k = Client::connect(&pipe, &token).unwrap();
    let _ = k.call("session.kill", json!({"id": id}));
}
