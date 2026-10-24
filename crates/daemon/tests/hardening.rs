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

#[test]
fn agent_role_cannot_use_app_only_calls() {
    let (pipe, token) = start("role");
    let mut app = Client::connect(&pipe, &token).unwrap();
    // a session that prints its own credential
    let (prog, args) = if cfg!(windows) {
        ("cmd", json!(["/c", "echo TOK[%AXELSTRA_AGENT_TOKEN%] & ping -n 20 127.0.0.1 >nul"]))
    } else {
        ("sh", json!(["-c", "echo TOK[$AXELSTRA_AGENT_TOKEN]; sleep 20"]))
    };
    let id = app.call("session.spawn", json!({"program": prog, "args": args})).unwrap()["id"].as_u64().unwrap();
    let t = Instant::now();
    let agent_token = loop {
        let lines = app.call("session.screen", json!({"id": id})).unwrap()["lines"].to_string();
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
    for (m, p) in [
        ("computer.enable", json!({"on": true})),
        ("project.add", json!({"path": std::env::temp_dir()})),
        ("project.setup", json!({"id": "x", "setup": "rm -rf /"})),
        ("hooks.install", json!({})),
        ("orch.start", json!({"project": "x"})),
    ] {
        let e = agent.call(m, p).unwrap_err().to_string();
        assert!(e.contains("not allowed to agents"), "{m}: {e}");
    }
    // ordinary calls still work, and a wrong credential is not accepted
    assert!(agent.call("computer.status", json!({})).is_ok());
    assert!(Client::connect_agent(&pipe, "bad", "nope").is_err());
    // the app role is unaffected
    assert_eq!(app.call("computer.enable", json!({"on": true})).unwrap()["enabled"], true);
}
