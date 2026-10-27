//! What a `role=agent` connection may do to other sessions and worktrees, and what the daemon accepts from files an
//! agent can write (the saved conversation id).
mod common;
use axelstra_daemon::client::Client;
use axelstra_daemon::server;
use serde_json::json;
use std::path::Path;
use std::time::{Duration, Instant};

fn serve(root: &Path, tag: &str, token: &str) -> String {
    let pipe = format!("{}-hagent-{tag}-{}", common::base_pipe(), std::process::id());
    let (p, t, s) = (pipe.clone(), token.to_string(), root.join("projects.json"));
    std::thread::spawn(move || {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap()
            .block_on(server::serve_with_store(p, t, s))
            .unwrap();
    });
    common::wait_ready(&pipe, token);
    pipe
}

fn temp(tag: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!("axelstra-hagent-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    root
}

/// Starts a long-lived shell that prints its own agent credential; returns the session id and a client that uses it.
fn agent_session(app: &mut Client, pipe: &str, extra: serde_json::Value) -> (u32, Client) {
    let (prog, args) = if cfg!(windows) {
        ("cmd", json!(["/q", "/k", "echo TOK[%AXELSTRA_AGENT_TOKEN%]"]))
    } else {
        ("sh", json!(["-c", "echo TOK[$AXELSTRA_AGENT_TOKEN]; exec cat"]))
    };
    let mut params = json!({"program": prog, "args": args});
    for (k, v) in extra.as_object().cloned().unwrap_or_default() {
        params[k] = v;
    }
    let id = app.call("session.spawn", params).unwrap()["id"].as_u64().unwrap() as u32;
    let t = Instant::now();
    let token = loop {
        let lines = screen(app, id);
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
    (id, Client::connect_agent(pipe, &"x".repeat(32), &token).unwrap())
}

fn screen(app: &mut Client, id: u32) -> String {
    app.call("session.screen", json!({"id": id, "lines": 100})).unwrap()["lines"].to_string()
}

#[test]
fn an_agent_drives_only_its_own_session_tree() {
    let root = temp("tree");
    let token = "f".repeat(32);
    let pipe = serve(&root, "tree", &token);
    let mut app = Client::connect(&pipe, &token).unwrap();
    // `a` started `b`; `c` is a stranger
    let (a, mut agent_a) = agent_session(&mut app, &pipe, json!({}));
    let (b, mut agent_b) = agent_session(&mut app, &pipe, json!({"parent": a}));
    let (c, _agent_c) = agent_session(&mut app, &pipe, json!({}));

    // `b` reaches neither its parent nor a stranger: no stream, no kill, no remove
    for target in [a, c] {
        for m in ["session.subscribe", "session.screen", "session.kill", "session.remove"] {
            let e = agent_b.call(m, json!({"id": target})).unwrap_err().to_string();
            assert!(e.contains("only allowed for the agent's own session"), "{m} {target}: {e}");
        }
    }
    // keystrokes are dropped too: neither a stranger nor the parent sees them
    agent_b.send_input(c, b"echo HOSTILE_KEYS_1\r").unwrap();
    agent_b.send_input(a, b"echo HOSTILE_KEYS_2\r").unwrap();
    // `a` types into its worker, and the text shows up there
    agent_a.send_input(b, b"echo LEAD_KEYS_3\r").unwrap();
    let t = Instant::now();
    while !screen(&mut app, b).contains("LEAD_KEYS_3") {
        assert!(t.elapsed() < Duration::from_secs(15), "the lead could not type into its worker: {}", screen(&mut app, b));
        std::thread::sleep(Duration::from_millis(200));
    }
    std::thread::sleep(Duration::from_millis(500));
    assert!(!screen(&mut app, c).contains("HOSTILE_KEYS_1"), "a worker typed into a stranger");
    assert!(!screen(&mut app, a).contains("HOSTILE_KEYS_2"), "a worker typed into its parent");
    // the lead may stream and close its worker; it may not close a stranger
    agent_a.call("session.subscribe", json!({"id": b})).unwrap();
    assert!(agent_a.call("session.screen", json!({"id": b})).is_ok());
    assert!(agent_a.call("session.screen", json!({"id": a})).is_ok());
    assert!(agent_a.call("session.kill", json!({"id": c})).is_err());
    agent_a.call("session.kill", json!({"id": b})).unwrap();
    // the user's own connection is not limited
    app.call("session.kill", json!({"id": c})).unwrap();
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_hostile_resume_id_is_not_handed_out() {
    let root = temp("resume");
    let wt = root.join("wt");
    std::fs::create_dir_all(&wt).unwrap();
    std::fs::create_dir_all(root.join("sid")).unwrap();
    let rec = |key: &str| {
        json!({"program": "cmd", "args": ["/q"], "cwd": wt.to_string_lossy(), "label": key,
               "meta": {"project": "p", "worktree": wt.to_string_lossy(), "kind": "agent", "agent": "claude", "key": key}})
    };
    let good = "0f8fad5b-d9cb-469f-a165-70867728950e";
    let hostile = ["x; calc", "$(id)", "abc\" -Command calc", "0f8fad5b-d9cb-469f-a165-70867728950e && calc"];
    let mut recs = vec![rec("good")];
    std::fs::write(root.join("sid/good.json"), json!({"session_id": good}).to_string()).unwrap();
    for (i, h) in hostile.iter().enumerate() {
        recs.push(rec(&format!("bad{i}")));
        std::fs::write(root.join(format!("sid/bad{i}.json")), json!({"session_id": h}).to_string()).unwrap();
    }
    std::fs::write(root.join("sessions.json"), json!({"v": 1, "data": recs}).to_string()).unwrap();
    let token = "e".repeat(32);
    let pipe = serve(&root, "resume", &token);
    let mut app = Client::connect(&pipe, &token).unwrap();
    let pending = app.call("session.pending", json!({})).unwrap();
    let by_key =
        |k: &str| pending.as_array().unwrap().iter().find(|r| r["meta"]["key"] == k).cloned().unwrap_or_else(|| panic!("{k} missing"));
    assert_eq!(by_key("good")["meta"]["resume"], good);
    for i in 0..hostile.len() {
        let r = by_key(&format!("bad{i}"));
        assert!(r["meta"]["resume"].is_null(), "a hostile session_id became a resume id: {}", r["meta"]["resume"]);
    }
    let _ = std::fs::remove_dir_all(&root);
}

fn git(cwd: &Path, args: &[&str]) {
    assert!(std::process::Command::new("git").current_dir(cwd).args(args).status().unwrap().success(), "git {args:?}");
}

#[test]
fn an_agent_removes_only_its_own_clean_integrated_worktree() {
    let root = temp("wtremove");
    let token = "b".repeat(32);
    let pipe = serve(&root, "wtremove", &token);
    let repo = root.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["config", "user.email", "t@t"]);
    git(&repo, &["config", "user.name", "t"]);
    std::fs::write(repo.join("a.txt"), "x").unwrap();
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-qm", "init"]);
    let mut app = Client::connect(&pipe, &token).unwrap();
    let id = app.call("project.add", json!({"path": &repo})).unwrap()["id"].as_str().unwrap().to_string();
    let mk = |app: &mut Client, name: &str| {
        app.call("worktree.create", json!({"project": id, "name": name})).unwrap()["path"].as_str().unwrap().to_string()
    };
    let (wa, wb, wc) = (mk(&mut app, "a"), mk(&mut app, "b"), mk(&mut app, "c"));
    let meta = |w: &str| json!({"project": id, "worktree": w});
    let (_, mut agent_a) = agent_session(&mut app, &pipe, json!({"cwd": wa, "meta": meta(&wa)}));
    let (_, mut agent_c) = agent_session(&mut app, &pipe, json!({"cwd": wc, "meta": meta(&wc)}));
    let count = |app: &mut Client| app.call("project.list", json!({})).unwrap()[0]["worktrees"].as_array().unwrap().len();
    assert_eq!(count(&mut app), 4);

    // another worktree is out of reach, clean or not
    let e = agent_a.call("worktree.remove", json!({"project": id, "path": wb})).unwrap_err().to_string();
    assert!(e.contains("own worktree"), "{e}");
    // its own, with an uncommitted file: refused, and nothing was touched
    std::fs::write(Path::new(&wa).join("new.txt"), "work").unwrap();
    let e = agent_a.call("worktree.remove", json!({"project": id, "path": wa})).unwrap_err().to_string();
    assert!(e.contains("uncommitted"), "{e}");
    // committed but not integrated: refused too
    git(Path::new(&wa), &["add", "new.txt"]);
    git(Path::new(&wa), &["commit", "-qm", "work"]);
    let e = agent_a.call("worktree.remove", json!({"project": id, "path": wa})).unwrap_err().to_string();
    assert!(e.contains("uncommitted"), "{e}");
    assert!(Path::new(&wa).join("new.txt").exists());
    assert_eq!(count(&mut app), 4);
    // the user's connection still forces it (the app warned the user)
    app.call("worktree.remove", json!({"project": id, "path": wa})).unwrap();
    // an agent in its own clean worktree with nothing of its own on the branch: allowed
    agent_c.call("worktree.remove", json!({"project": id, "path": wc})).unwrap();
    assert!(!Path::new(&wc).exists());
    assert_eq!(count(&mut app), 2);
    let _ = std::fs::remove_dir_all(&root);
}
