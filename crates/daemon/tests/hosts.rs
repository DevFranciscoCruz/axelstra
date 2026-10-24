//! The host registry (`hosts` in settings) over a real daemon, on an isolated data dir (docs/TESTING.md).
mod common;
use axelstra_daemon::client::Client;
use axelstra_daemon::server;
use serde_json::{json, Value};
use std::time::{Duration, Instant};

fn start(tag: &str) -> (String, String) {
    let root = common::store(tag).parent().unwrap().to_path_buf();
    let pipe = common::pipe(tag);
    let token = "h".repeat(32);
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

/// A connection with role=agent: a session prints its own credential, which the test then presents.
fn agent_of(app: &mut Client, pipe: &str) -> Client {
    let (prog, args) = if cfg!(windows) {
        ("cmd", json!(["/c", "echo TOK[%AXELSTRA_AGENT_TOKEN%] & ping -n 20 127.0.0.1 >nul"]))
    } else {
        ("sh", json!(["-c", "echo TOK[$AXELSTRA_AGENT_TOKEN]; sleep 20"]))
    };
    let id = app.call("session.spawn", json!({"program": prog, "args": args})).unwrap()["id"].as_u64().unwrap();
    let t = Instant::now();
    let tok = loop {
        let lines = app.call("session.screen", json!({"id": id})).unwrap()["lines"].to_string();
        if let Some(i) = lines.find("TOK[") {
            let rest = &lines[i + 4..];
            if let Some(j) = rest.find(']').filter(|j| *j >= 32) {
                break rest[..j].to_string();
            }
        }
        assert!(t.elapsed() < Duration::from_secs(15), "no token printed: {lines}");
        std::thread::sleep(Duration::from_millis(200));
    };
    Client::connect_agent(pipe, &"x".repeat(32), &tok).unwrap()
}

#[test]
fn hosts_round_trip_and_invalid_ones_save_nothing() {
    let (pipe, token) = start("hosts-rt");
    let mut app = Client::connect(&pipe, &token).unwrap();
    assert_eq!(app.call("settings.get", json!({})).unwrap()["hosts"], json!([]));

    let good = json!([
        {"id": "box", "name": "Box", "spec": " ssh:me@box:2222 ", "slot": 1, "bin": "~/bin/axelstra", "junk": true},
        {"id": "wsl", "name": "WSL", "spec": "wsl:Ubuntu", "slot": 2}
    ]);
    app.call("settings.set", json!({"hosts": good})).unwrap();
    let saved = app.call("settings.get", json!({})).unwrap()["hosts"].clone();
    assert_eq!(
        saved,
        json!([
            {"id": "box", "name": "Box", "spec": "ssh:me@box:2222", "slot": 1, "bin": "~/bin/axelstra"},
            {"id": "wsl", "name": "WSL", "spec": "wsl:Ubuntu", "slot": 2}
        ])
    );

    // other settings keys leave the registry alone
    app.call("settings.set", json!({"notify_sound": false})).unwrap();
    assert_eq!(app.call("settings.get", json!({})).unwrap()["hosts"], saved);

    // an invalid list is refused whole and the saved one stays
    let bad: [Value; 5] = [
        json!([{"id": "local", "name": "L", "spec": "wsl:U", "slot": 3}]),
        json!([{"id": "a", "name": "A", "spec": "ssh:-oProxyCommand=x", "slot": 3}]),
        json!([{"id": "a", "name": "A", "spec": "wsl:U", "slot": 1}, {"id": "b", "name": "B", "spec": "wsl:U", "slot": 1}]),
        json!([{"id": "a", "name": "A", "spec": "wsl:U", "slot": 128}]),
        json!("box"),
    ];
    for b in bad {
        assert!(app.call("settings.set", json!({"hosts": b.clone(), "notify_sound": true})).is_err(), "{b}");
        assert_eq!(app.call("settings.get", json!({})).unwrap()["hosts"], saved, "{b}");
        // nothing else in the same request was saved either
        assert_eq!(app.call("settings.get", json!({})).unwrap()["notify_sound"], false);
    }

    // null and [] clear
    app.call("settings.set", json!({"hosts": null})).unwrap();
    assert_eq!(app.call("settings.get", json!({})).unwrap()["hosts"], json!([]));
    app.call("settings.set", json!({"hosts": good})).unwrap();
    app.call("settings.set", json!({"hosts": []})).unwrap();
    assert_eq!(app.call("settings.get", json!({})).unwrap()["hosts"], json!([]));
}

#[test]
fn an_agent_cannot_change_the_host_registry() {
    let (pipe, token) = start("hosts-role");
    let mut app = Client::connect(&pipe, &token).unwrap();
    let mut agent = agent_of(&mut app, &pipe);
    for p in [json!({"hosts": []}), json!({"hosts": null}), json!({"hosts": [{"id": "a", "name": "A", "spec": "wsl:U", "slot": 1}]})] {
        let e = agent.call("settings.set", p).unwrap_err().to_string();
        assert!(e.contains("not allowed to agents"), "{e}");
    }
    assert_eq!(app.call("settings.get", json!({})).unwrap()["hosts"], json!([]));
    // an agent may still read them and change other settings
    assert!(agent.call("settings.get", json!({})).is_ok());
    assert!(agent.call("settings.set", json!({"notify_sound": false})).is_ok());
}
