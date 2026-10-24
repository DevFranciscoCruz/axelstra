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
        ("backup.export", json!({})),
        ("backup.restore", json!({"path": "x"})),
        ("workspace.export", json!({"path": "x"})),
        ("workspace.import_preview", json!({"path": "x"})),
        ("workspace.import", json!({"path": "x", "mode": "replace"})),
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

/// Child half of `a_process_started_by_a_session_cannot_get_the_user_role`: runs inside the session, knows the master
/// token (handed over in its env, the most a process reading the data dir could ever learn) and uses it with no
/// agent credential at all, as a hostile agent would.
#[test]
fn session_probe_child() {
    let Ok(out) = std::env::var("AXELSTRA_PROBE_OUT") else { return };
    let (pipe, token) = (std::env::var("AXELSTRA_PROBE_PIPE").unwrap(), std::env::var("AXELSTRA_PROBE_TOKEN").unwrap());
    let mut c = Client::connect(&pipe, &token).unwrap();
    let mut report = vec![];
    for (m, p) in
        [("computer.enable", json!({"on": true})), ("project.add", json!({"path": std::env::temp_dir()})), ("hooks.install", json!({}))]
    {
        report.push(format!("{m}: {}", c.call(m, p).err().map(|e| e.to_string()).unwrap_or_else(|| "ALLOWED".into())));
    }
    std::fs::write(out, report.join("\n")).unwrap();
}

#[test]
fn a_process_started_by_a_session_cannot_get_the_user_role() {
    let (pipe, token) = start("origin");
    let mut app = Client::connect(&pipe, &token).unwrap();
    let out = std::env::temp_dir().join(format!("axelstra-probe-{}.txt", std::process::id()));
    let _ = std::fs::remove_file(&out);
    let exe = std::env::current_exe().unwrap();
    let env = json!({
        "AXELSTRA_PROBE_OUT": out.to_string_lossy(),
        "AXELSTRA_PROBE_PIPE": pipe,
        "AXELSTRA_PROBE_TOKEN": token,
    });
    app.call(
        "session.spawn",
        json!({"program": exe.to_string_lossy(), "args": ["session_probe_child", "--exact", "--nocapture", "--test-threads=1"], "env": env}),
    )
    .unwrap();
    let t = Instant::now();
    let report = loop {
        if let Ok(r) = std::fs::read_to_string(&out) {
            break r;
        }
        assert!(t.elapsed() < Duration::from_secs(30), "the probe never reported");
        std::thread::sleep(Duration::from_millis(200));
    };
    assert_eq!(report.lines().count(), 3, "{report}");
    for l in report.lines() {
        assert!(l.contains("not allowed to agents"), "a session's process got the user role: {l}");
    }
    // the same token from outside every session is the user's
    assert_eq!(app.call("computer.enable", json!({"on": true})).unwrap()["enabled"], true);
}

#[test]
fn computer_actions_are_refused_by_the_daemon_while_screen_control_is_off() {
    let (pipe, token) = start("compgate");
    // even the user's own connection and a hand-written client (no CLI in between) are refused
    let mut c = Client::connect(&pipe, &token).unwrap();
    for (m, p) in [
        ("computer.windows", json!({})),
        ("computer.screenshot", json!({})),
        ("computer.click", json!({"x": 1, "y": 1})),
        ("computer.type", json!({"text": "x"})),
        ("computer.key", json!({"combo": "enter"})),
        ("computer.scroll", json!({"dy": 1})),
        ("computer.focus", json!({"window": 1})),
    ] {
        let e = c.call(m, p).unwrap_err().to_string();
        assert!(e.contains("screen control is off"), "{m}: {e}");
    }
    assert_eq!(c.call("computer.status", json!({})).unwrap()["enabled"], false);
}
