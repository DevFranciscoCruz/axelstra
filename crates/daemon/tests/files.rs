use axelstra_daemon::client::Client;
use axelstra_daemon::server;
use serde_json::json;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

fn sh(cwd: &Path, args: &[&str]) {
    assert!(Command::new("git").current_dir(cwd).args(args).status().unwrap().success(), "git {args:?}");
}

#[test]
fn list_read_write_and_confinement() {
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(90));
        eprintln!("watchdog: test stuck");
        std::process::exit(1);
    });

    let root = std::env::temp_dir().join(format!("axelstra-fs-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let repo = root.join("r");
    std::fs::create_dir_all(repo.join("src")).unwrap();
    std::fs::create_dir_all(repo.join("node_modules/x")).unwrap();
    sh(&repo, &["init", "-q", "-b", "main"]);
    sh(&repo, &["config", "user.email", "t@t"]);
    sh(&repo, &["config", "user.name", "t"]);
    std::fs::write(repo.join(".gitignore"), "node_modules/\n*.log\n").unwrap();
    std::fs::write(repo.join("src/a.rs"), "fn a() {}\r\nfn b() {}\r\n").unwrap();
    std::fs::write(repo.join("b.txt"), "plain\n").unwrap();
    std::fs::write(repo.join("debug.log"), "noise").unwrap();
    std::fs::write(repo.join("bin.dat"), [0u8, 1, 2]).unwrap();
    sh(&repo, &["add", "-A"]);
    sh(&repo, &["commit", "-qm", "init"]);

    let pipe = format!("{}-fs-{}", axelstra_daemon::proto::pipe_name(), std::process::id());
    let token = "f".repeat(32);
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
    let mut c = Client::connect(&pipe, &token).unwrap();
    c.call("project.add", json!({"path": &repo})).unwrap();
    let list = c.call("project.list", json!({})).unwrap();
    let w = list[0]["worktrees"][0]["path"].clone();

    // listing: folders first, .gitignore honoured, .git hidden
    let e = c.call("fs.list", json!({"worktree": w, "path": ""})).unwrap();
    let names: Vec<&str> = e["entries"].as_array().unwrap().iter().map(|x| x["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["src", ".gitignore", "b.txt", "bin.dat"], "{names:?}");
    let e = c.call("fs.list", json!({"worktree": w, "path": "src"})).unwrap();
    assert_eq!(e["entries"][0]["name"], "a.rs");

    // CRLF files edit as LF and are written back as CRLF
    let o = c.call("fs.read", json!({"worktree": w, "path": "src/a.rs"})).unwrap();
    assert_eq!((o["eol"].as_str(), o["text"].as_str()), (Some("crlf"), Some("fn a() {}\nfn b() {}\n")));
    let hash = o["hash"].as_str().unwrap().to_string();
    let r = c
        .call("fs.write", json!({"worktree": w, "path": "src/a.rs", "text": "fn a() {}\nfn c() {}\n", "eol": "crlf", "base_hash": hash}))
        .unwrap();
    assert!(r["hash"].is_string(), "{r}");
    assert_eq!(std::fs::read(repo.join("src/a.rs")).unwrap(), b"fn a() {}\r\nfn c() {}\r\n");

    // a stale base_hash (agent changed the file meanwhile) is a conflict, nothing is overwritten
    std::fs::write(repo.join("src/a.rs"), "changed by agent\r\n").unwrap();
    let r = c.call("fs.write", json!({"worktree": w, "path": "src/a.rs", "text": "mine\n", "eol": "crlf", "base_hash": hash})).unwrap();
    assert_eq!(r["conflict"], true);
    assert_eq!(std::fs::read(repo.join("src/a.rs")).unwrap(), b"changed by agent\r\n");
    assert!(c.call("fs.stat", json!({"worktree": w, "path": "src/a.rs"})).unwrap()["hash"].is_string());
    assert!(c.call("fs.stat", json!({"worktree": w, "path": "nope.txt"})).unwrap()["hash"].is_null());

    // new file: no base_hash needed
    let r = c.call("fs.write", json!({"worktree": w, "path": "src/new.txt", "text": "hi\n", "eol": "lf"})).unwrap();
    assert!(r["hash"].is_string());

    // refusals
    assert!(c.call("fs.read", json!({"worktree": w, "path": "bin.dat"})).is_err());
    assert!(c.call("fs.read", json!({"worktree": w, "path": "../../x"})).is_err());
    assert!(c.call("fs.read", json!({"worktree": w, "path": "/etc/passwd"})).is_err());
    assert!(c.call("fs.read", json!({"worktree": w, "path": r"..\..\x"})).is_err());
    assert!(c.call("fs.read", json!({"worktree": w, "path": r"C:\Windows\win.ini"})).is_err());
    assert!(c.call("fs.write", json!({"worktree": w, "path": "../evil.txt", "text": "x", "eol": "lf"})).is_err());
    // backslash is only a separator on Windows; elsewhere it is an ordinary (harmless) file name
    #[cfg(windows)]
    assert!(c.call("fs.write", json!({"worktree": w, "path": r"..\evil.txt", "text": "x", "eol": "lf"})).is_err());
    assert!(c.call("fs.list", json!({"worktree": root.to_string_lossy(), "path": ""})).is_err());
}
