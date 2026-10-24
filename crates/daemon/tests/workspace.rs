use axelstra_daemon::client::Client;
use axelstra_daemon::server;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::Duration;

fn start(root: &Path, tag: &str) -> Client {
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(90));
        eprintln!("watchdog: test stuck");
        std::process::exit(1);
    });
    let pipe = format!("{}-ws-{tag}-{}", axelstra_daemon::proto::pipe_name(), std::process::id());
    let token = "w".repeat(32);
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
    Client::connect(&pipe, &token).unwrap()
}

fn fresh(tag: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("axelstra-ws-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    root
}

fn card(id: &str) -> String {
    format!("---\nid: {id}\ntitle: T\nstatus: todo\n---\n\nbody {id}\n")
}

#[test]
fn export_on_one_daemon_import_on_another_with_a_folder_map() {
    let root = fresh("move");
    let (a, b) = (root.join("a"), root.join("b"));
    let (repo_a, repo_b) = (a.join("repo"), b.join("repo"));
    for d in [&repo_a, &repo_b, &a, &b] {
        std::fs::create_dir_all(d).unwrap();
    }
    std::fs::write(a.join("token"), "MASTER-SECRET").unwrap();
    std::fs::create_dir_all(repo_a.join(".kanban")).unwrap();
    std::fs::write(repo_a.join(".kanban/one.md"), card("one")).unwrap();

    let mut ca = start(&a, "a");
    ca.call("project.add", json!({"path": &repo_a})).unwrap();
    ca.call("settings.set", json!({"terse": "short", "notify_mode": "waiting"})).unwrap();
    let file = root.join("ws.json");
    let r = ca.call("workspace.export", json!({"path": &file})).unwrap();
    assert_eq!(r["summary"]["projects"], 1);
    assert_eq!(r["summary"]["cards"], 1);
    assert!(!std::fs::read_to_string(&file).unwrap().contains("MASTER-SECRET"));
    // the data folder is refused as a target
    assert!(ca.call("workspace.export", json!({"path": a.join("token")})).is_err());

    let mut cb = start(&b, "b");
    let map = json!([[repo_a.to_string_lossy(), repo_b.to_string_lossy()]]);
    let pv = cb.call("workspace.import_preview", json!({"path": &file, "map": map})).unwrap();
    assert_eq!(pv["projects"]["new"].as_array().unwrap().len(), 1);
    assert_eq!(pv["cards"]["new"].as_array().unwrap().len(), 1);
    assert!(cb.call("project.list", json!({})).unwrap().as_array().unwrap().is_empty(), "preview wrote");

    // bad parameters write nothing
    assert!(cb.call("workspace.import", json!({"path": &file, "mode": "wipe"})).is_err());
    assert!(cb.call("workspace.import", json!({"path": &file, "conflicts": "maybe"})).is_err());
    assert!(cb.call("workspace.import", json!({"path": root.join("missing.json")})).is_err());
    assert!(cb.call("workspace.import_preview", json!({"path": &file, "map": "nope"})).is_err());

    let out = cb.call("workspace.import", json!({"path": &file, "mode": "merge", "conflicts": "keep", "map": map})).unwrap();
    assert_eq!(out["projects_added"], 1);
    assert_eq!(out["cards_written"], 1);
    let list: Value = cb.call("project.list", json!({})).unwrap();
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert_eq!(cb.call("settings.get", json!({})).unwrap()["terse"], "short");
    assert!(repo_b.join(".kanban/one.md").is_file());
    // a second import has nothing new and keeps the card it finds
    let again = cb.call("workspace.import", json!({"path": &file, "map": map})).unwrap();
    assert_eq!((again["projects_added"].as_u64(), again["cards_written"].as_u64()), (Some(0), Some(0)));
}

#[test]
fn a_hostile_file_is_refused_and_leaves_the_state_alone() {
    let root = fresh("hostile");
    let mut c = start(&root, "h");
    c.call("settings.set", json!({"terse": "short"})).unwrap();
    let f = root.join("evil.json");
    for body in [
        "{ nope".to_string(),
        json!({"kind": "axelstra-workspace", "version": 99}).to_string(),
        json!({"kind": "axelstra-workspace", "version": 1, "projects": [{"id": "p", "name": "n", "path": "/x"}], "kanban": {"p": [{"file": "../evil.md", "text": "x"}]}}).to_string(),
    ] {
        std::fs::write(&f, body).unwrap();
        assert!(c.call("workspace.import_preview", json!({"path": &f})).is_err());
        for mode in ["merge", "replace"] {
            assert!(c.call("workspace.import", json!({"path": &f, "mode": mode, "conflicts": "overwrite"})).is_err());
        }
    }
    assert_eq!(c.call("settings.get", json!({})).unwrap()["terse"], "short");
    assert!(!root.join("projects.json.1").exists());
}
