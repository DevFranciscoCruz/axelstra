//! lag.log is written by events only: a slow call appends one line, an idle daemon appends nothing.
//! `debug.load` (only with AXELSTRA_TESTING) reports calls per method for tools/idle-load.mjs.
mod common;
use axelstra_daemon::client::Client;
use axelstra_daemon::server;
use serde_json::json;
use std::time::Duration;

#[test]
fn lag_log_has_no_periodic_writer_and_calls_are_counted() {
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(120));
        eprintln!("watchdog: test stuck");
        std::process::exit(1);
    });
    let data = common::data_dir();
    let log = data.join("lag.log");
    let _ = std::fs::remove_file(&log);
    let root = std::env::temp_dir().join(format!("axelstra-lagidle-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let pipe = format!("{}-lagidle-{}", common::base_pipe(), std::process::id());
    let token = "i".repeat(32);
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

    // nobody talks to the daemon: nothing is written
    std::thread::sleep(Duration::from_secs(3));
    assert!(!log.exists(), "idle daemon wrote lag.log: {:?}", std::fs::read_to_string(&log));

    let mut c = Client::connect(&pipe, &token).unwrap();
    c.call("ping", json!({})).unwrap();
    assert!(!log.exists(), "a fast call wrote lag.log");
    c.call("debug.slow", json!({"ms": 150})).unwrap();
    let text = std::fs::read_to_string(&log).expect("a slow call is logged");
    assert_eq!(text.lines().filter(|l| l.contains("daemon debug.slow")).count(), 1, "{text}");

    let load = c.call("debug.load", json!({})).unwrap();
    assert!(load["calls"]["ping"]["calls"].as_u64().unwrap() >= 1, "{load}");
    assert_eq!(load["calls"]["debug.slow"]["calls"], 1, "{load}");
    assert!(load["spawns"].is_u64());
}
