use axelstra_daemon::client::Client;
use axelstra_daemon::server;
use serde_json::json;
use std::io::Read;
use std::time::{Duration, Instant};

fn start(tag: &str) -> (String, String) {
    let root = std::env::temp_dir().join(format!("axelstra-hard-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let pipe = format!("{}-hard-{tag}-{}", axelstra_daemon::proto::pipe_name(), std::process::id());
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
    std::thread::sleep(Duration::from_millis(300));
    (pipe, token)
}

#[test]
fn only_the_app_turns_screen_control_on() {
    let (pipe, token) = start("computer");
    let mut cli = Client::connect_cli(&pipe, &token).unwrap();
    let e = cli.call("computer.enable", json!({"on": true})).unwrap_err().to_string();
    assert!(e.contains("only be turned on by the user"), "{e}");
    assert_eq!(cli.call("computer.status", json!({})).unwrap()["enabled"], false);
    // the app (no CLI marker) can, and anyone can turn it off again
    let mut app = Client::connect(&pipe, &token).unwrap();
    assert_eq!(app.call("computer.enable", json!({"on": true})).unwrap()["enabled"], true);
    assert_eq!(cli.call("computer.status", json!({})).unwrap()["enabled"], true);
    assert_eq!(cli.call("computer.enable", json!({"on": false})).unwrap()["enabled"], false);
}

#[test]
fn a_silent_connection_is_dropped() {
    let (pipe, _token) = start("silent");
    #[cfg(windows)]
    let mut conn = std::fs::OpenOptions::new().read(true).write(true).open(&pipe).unwrap();
    #[cfg(unix)]
    let mut conn = std::os::unix::net::UnixStream::connect(&pipe).unwrap();
    let t = Instant::now();
    // the daemon closes it without ever receiving a hello
    let mut b = [0u8; 1];
    let n = conn.read(&mut b).unwrap_or(0);
    assert_eq!(n, 0);
    assert!(t.elapsed() >= Duration::from_secs(5) && t.elapsed() < Duration::from_secs(30), "{:?}", t.elapsed());
}
