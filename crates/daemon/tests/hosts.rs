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
    common::wait_ready(&pipe, &token);
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

fn names(r: &Value) -> Vec<(String, bool)> {
    r["entries"].as_array().unwrap().iter().map(|e| (e["name"].as_str().unwrap().to_string(), e["git"] == true)).collect()
}

#[test]
fn host_info_and_browse() {
    let (pipe, token) = start("hosts-browse");
    let mut app = Client::connect(&pipe, &token).unwrap();

    let info = app.call("host.info", json!({})).unwrap();
    let os = if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "linux"
    };
    assert_eq!(info["os"], os);
    assert_eq!(info["sep"], std::path::MAIN_SEPARATOR.to_string());
    assert_eq!(info["proto"], axelstra_daemon::proto::PROTO);
    assert_eq!(info["build"], axelstra_daemon::proto::BUILD_ID);
    assert_eq!(info["version"], axelstra_core::buildinfo::VERSION);
    assert_eq!(info["commit"], axelstra_core::buildinfo::COMMIT);
    assert_eq!(info["built"], axelstra_core::buildinfo::BUILT);
    assert!(info["home"].is_string() && info["user"].is_string() && info["hostname"].is_string());
    assert_eq!(info["shell"].is_null(), cfg!(windows) || std::env::var_os("SHELL").is_none());

    // default: home
    let home = app.call("host.browse", json!({})).unwrap();
    assert_eq!(home["path"], info["home"]);
    assert_eq!(home["home"], info["home"]);
    assert!(!home["path"].as_str().unwrap().starts_with(r"\?\"));
    assert!(home["entries"].is_array());

    // a temp dir with subdirs, a repo, a file and a dot dir
    let d = std::env::temp_dir().join(format!("axelstra-browse-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    for n in ["beta", "Alpha", "repo/.git", "gamma"] {
        std::fs::create_dir_all(d.join(n)).unwrap();
    }
    std::fs::write(d.join("file.txt"), "x").unwrap();
    // the daemon answers with canonical paths (macOS /var -> /private/var, Windows 8.3 names), so compare against those
    let d = axelstra_core::git::canon(&d).unwrap();
    let r = app.call("host.browse", json!({"path": d.to_string_lossy()})).unwrap();
    assert_eq!(names(&r), [("Alpha".to_string(), false), ("beta".into(), false), ("gamma".into(), false), ("repo".into(), true)]);
    assert!(r.get("truncated").is_none());
    assert!(!r["path"].as_str().unwrap().starts_with(r"\?\"));
    let parent = r["parent"].as_str().unwrap();
    assert!(d.starts_with(parent), "{parent}");
    // a trailing separator or a dotted path reaches the same directory
    let dotted = d.join("beta").join("..");
    assert_eq!(app.call("host.browse", json!({"path": dotted.to_string_lossy()})).unwrap()["path"], r["path"]);

    // a directory above the limit is cut
    let big = d.join("big");
    for i in 0..510 {
        std::fs::create_dir_all(big.join(format!("d{i:03}"))).unwrap();
    }
    let r = app.call("host.browse", json!({"path": big.to_string_lossy()})).unwrap();
    assert_eq!(r["entries"].as_array().unwrap().len(), 500);
    assert_eq!(r["truncated"], true);

    // errors
    let e = app.call("host.browse", json!({"path": d.join("nope").to_string_lossy()})).unwrap_err().to_string();
    assert!(e.contains("does not exist"), "{e}");
    let e = app.call("host.browse", json!({"path": d.join("file.txt").to_string_lossy()})).unwrap_err().to_string();
    assert!(e.contains("not a directory"), "{e}");
    let e = app.call("host.browse", json!({"path": "relative/dir"})).unwrap_err().to_string();
    assert!(e.contains("not an absolute path"), "{e}");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn an_agent_cannot_look_around_the_host() {
    let (pipe, token) = start("hosts-browse-role");
    let mut app = Client::connect(&pipe, &token).unwrap();
    let mut agent = agent_of(&mut app, &pipe);
    for (m, p) in [("host.info", json!({})), ("host.browse", json!({})), ("host.browse", json!({"path": std::env::temp_dir()}))] {
        let e = agent.call(m, p).unwrap_err().to_string();
        assert!(e.contains("not allowed to agents"), "{m}: {e}");
    }
    assert!(app.call("host.info", json!({})).is_ok());
}
