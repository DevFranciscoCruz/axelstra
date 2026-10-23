use axelstra_daemon::client::Client;
use axelstra_daemon::server;
use serde_json::json;
use std::time::Duration;

#[test]
fn rename_persists_with_the_session() {
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(60));
        eprintln!("watchdog: test stuck");
        std::process::exit(1);
    });
    let root = std::env::temp_dir().join(format!("axelstra-rename-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let pipe = format!("{}-rename-{}", axelstra_daemon::proto::pipe_name(), std::process::id());
    let token = "r".repeat(32);
    let (p, t, s) = (pipe.clone(), token.clone(), root.join("projects.json"));
    std::thread::spawn(move || {
        tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap().block_on(server::serve_with_store(p, t, s)).unwrap();
    });
    std::thread::sleep(Duration::from_millis(300));
    let mut c = Client::connect(&pipe, &token).unwrap();

    let sid = c
        .call(
            "session.spawn",
            json!({"program": if cfg!(windows) { "cmd" } else { "sh" }, "args": if cfg!(windows) { vec!["/q"] } else { vec![] }, "label": "shell", "meta": {"project": "p"}}),
        )
        .unwrap()["id"]
        .as_u64()
        .unwrap();

    let r = c.call("session.rename", json!({"id": sid, "label": "  build  "})).unwrap();
    assert_eq!(r["label"], "build");
    let list = c.call("session.list", json!({})).unwrap();
    assert_eq!(list[0]["label"], "build");
    // the saved restore data carries the new name
    let saved: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(root.join("sessions.json")).unwrap()).unwrap();
    assert!(saved.as_array().unwrap().iter().any(|s| s["label"] == "build"));

    // empty names and unknown sessions are refused, and the name is left alone
    assert!(c.call("session.rename", json!({"id": sid, "label": "   "})).is_err());
    assert!(c.call("session.rename", json!({"id": sid + 99, "label": "x"})).is_err());
    assert_eq!(c.call("session.list", json!({})).unwrap()[0]["label"], "build");

    c.call("session.remove", json!({"id": sid})).unwrap();
}
