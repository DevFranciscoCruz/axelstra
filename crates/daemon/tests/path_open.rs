//! `files.resolve_path` / `files.path_action`: the daemon half of Ctrl+click on a path that is not code or html
//! (rules in `pathopen.rs`). `AXELSTRA_TEST_OPENER_LOG` keeps every action away from the real system.
mod common;
use axelstra_daemon::client::Client;
use axelstra_daemon::server;
use serde_json::json;

fn start(root: &std::path::Path) -> Client {
    let pipe = format!("{}-popen-{}", common::base_pipe(), std::process::id());
    let token = "p".repeat(32);
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
    Client::connect(&pipe, &token).unwrap()
}

fn code(e: &anyhow::Error) -> Option<String> {
    axelstra_core::msg::find(e).map(|c| c.code.clone())
}

#[test]
fn describes_paths_and_acts_only_as_allowed() {
    let root = std::env::temp_dir().join(format!("axelstra-popen-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let (wt, other) = (root.join("proj"), root.join("proj-evil"));
    std::fs::create_dir_all(wt.join("target/release/bundle dir")).unwrap();
    std::fs::create_dir_all(&other).unwrap();
    std::fs::write(wt.join("target/release/bundle dir/Setup_0.1.0.exe"), "MZ").unwrap();
    std::fs::write(wt.join("photo.png"), "x").unwrap();
    std::fs::write(other.join("evil.exe"), "MZ").unwrap();
    let log = root.join("opener.log");
    std::env::set_var("AXELSTRA_TEST_OPENER_LOG", &log);
    let mut c = start(&root);
    let (prog, args) = if cfg!(windows) { ("cmd", json!(["/q"])) } else { ("sh", json!([])) };
    let id = c.call("session.spawn", json!({"program": prog, "args": args, "cwd": wt})).unwrap()["id"].as_u64().unwrap();

    let r = c.call("files.resolve_path", json!({"session": id, "path": "target/release/bundle dir/Setup_0.1.0.exe:3"})).unwrap();
    assert_eq!(
        (r["exec"].as_bool(), r["can_run"].as_bool(), r["can_open"].as_bool(), r["inside"].as_bool()),
        (Some(true), Some(true), Some(false), Some(true)),
        "{r}"
    );
    let exe = r["path"].as_str().unwrap().to_string();
    assert!(!exe.starts_with(r"\\?\"), "{r}");
    let r = c.call("files.resolve_path", json!({"session": id, "path": "photo.png"})).unwrap();
    assert_eq!((r["can_run"].as_bool(), r["can_open"].as_bool()), (Some(false), Some(true)), "{r}");

    // running goes to the (simulated) opener with the canonical path; opening a program is refused
    c.call("files.path_action", json!({"session": id, "path": "target/release/bundle dir/Setup_0.1.0.exe", "action": "run"})).unwrap();
    let e = c
        .call("files.path_action", json!({"session": id, "path": "target/release/bundle dir/Setup_0.1.0.exe", "action": "open"}))
        .unwrap_err();
    assert_eq!(code(&e).as_deref(), Some("files.path_not_allowed"), "{e:#}");
    // outside the session roots: reveal only
    let evil = other.join("evil.exe");
    let e = c.call("files.path_action", json!({"session": id, "path": evil, "action": "run"})).unwrap_err();
    assert_eq!(code(&e).as_deref(), Some("files.path_not_allowed"), "{e:#}");
    c.call("files.path_action", json!({"session": id, "path": evil, "action": "reveal"})).unwrap();
    // missing, unknown action, unknown session
    let e = c.call("files.resolve_path", json!({"session": id, "path": "nope.exe"})).unwrap_err();
    assert_eq!(code(&e).as_deref(), Some("files.path_not_found"), "{e:#}");
    assert!(c.call("files.path_action", json!({"session": id, "path": "photo.png", "action": "format"})).is_err());
    let e = c.call("files.resolve_path", json!({"session": 987654, "path": "x"})).unwrap_err();
    assert!(format!("{e:#}").contains("no such session"), "{e:#}");

    let text = std::fs::read_to_string(&log).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 2, "{text}");
    assert_eq!(lines[0], format!("run\t{exe}"));
    assert!(lines[1].starts_with("reveal\t") && lines[1].ends_with("evil.exe"), "{text}");
    // the run is in the audit log, as a daemon entry
    let a = c.call("audit.list", json!({"limit": 50})).unwrap();
    assert!(a.to_string().contains("files.run_path"), "{a}");

    std::env::remove_var("AXELSTRA_TEST_OPENER_LOG");
    let _ = c.call("session.kill", json!({"id": id}));
    let _ = std::fs::remove_dir_all(&root);
}
