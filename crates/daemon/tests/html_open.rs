//! `files.resolve_html`: the daemon half of Ctrl+click on an HTML path in a terminal (rules in `htmlpath.rs`).
mod common;
use axelstra_daemon::client::Client;
use axelstra_daemon::server;
use serde_json::json;

fn start(root: &std::path::Path) -> Client {
    let pipe = format!("{}-html-{}", common::base_pipe(), std::process::id());
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
    Client::connect(&pipe, &token).unwrap()
}

#[test]
fn resolves_against_the_session_cwd_and_confines_relative_paths() {
    let root = std::env::temp_dir().join(format!("axelstra-html-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let (wt, other) = (root.join("proj"), root.join("proj-evil"));
    std::fs::create_dir_all(wt.join("out dir")).unwrap();
    std::fs::create_dir_all(&other).unwrap();
    std::fs::write(wt.join("out dir/report.html"), "<p>").unwrap();
    std::fs::write(other.join("e.html"), "<p>").unwrap();
    let mut c = start(&root);
    let (prog, args) = if cfg!(windows) { ("cmd", json!(["/q"])) } else { ("sh", json!([])) };
    let id = c.call("session.spawn", json!({"program": prog, "args": args, "cwd": wt})).unwrap()["id"].as_u64().unwrap();

    let r = c.call("files.resolve_html", json!({"session": id, "path": "out dir/report.html:12:3"})).unwrap();
    assert!(r["url"].as_str().unwrap().starts_with("file:///"), "{r}");
    assert!(r["url"].as_str().unwrap().ends_with("/out%20dir/report.html"), "{r}");
    assert!(!r["path"].as_str().unwrap().starts_with(r"\?\"), "{r}");

    // absolute and explicit: allowed outside the session folder
    assert!(c.call("files.resolve_html", json!({"session": id, "path": other.join("e.html")})).is_ok());
    // relative that leaves the folder: refused, with a code
    let e = c.call("files.resolve_html", json!({"session": id, "path": "../proj-evil/e.html"})).unwrap_err();
    assert_eq!(axelstra_core::msg::find(&e).map(|c| c.code.as_str()), Some("files.html_not_allowed"), "{e:#}");
    let e = c.call("files.resolve_html", json!({"session": id, "path": "missing.html"})).unwrap_err();
    assert_eq!(axelstra_core::msg::find(&e).map(|c| c.code.as_str()), Some("files.html_not_found"), "{e:#}");
    let e = c.call("files.resolve_html", json!({"session": 987654, "path": "x.html"})).unwrap_err();
    assert!(format!("{e:#}").contains("no such session"), "{e:#}");
    let _ = c.call("session.kill", json!({"id": id}));
    let _ = std::fs::remove_dir_all(&root);
}
