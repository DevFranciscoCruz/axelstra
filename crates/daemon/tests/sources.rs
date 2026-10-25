//! Add Project sources over a real daemon on an isolated data dir (docs/TESTING.md): clones come from local bare
//! repositories in temp dirs, never the network.
mod common;
use axelstra_daemon::client::Client;
use axelstra_daemon::server;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

fn start(tag: &str) -> (String, String) {
    let root = common::store(tag).parent().unwrap().to_path_buf();
    let pipe = common::pipe(tag);
    let token = "s".repeat(32);
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

fn git(dir: &Path, args: &[&str]) {
    let o = Command::new("git")
        .current_dir(dir)
        .args(["-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false"])
        .args(args)
        .output()
        .unwrap();
    assert!(o.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&o.stderr));
}

fn tmp(tag: &str) -> PathBuf {
    let d = common::data_dir().join(format!("src-{tag}"));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A bare repository with one commit on `main`, as a `file://` URL.
fn bare_repo(base: &Path) -> String {
    let work = base.join("work");
    std::fs::create_dir_all(&work).unwrap();
    git(&work, &["init", "-b", "main"]);
    std::fs::write(work.join("a.txt"), "hi").unwrap();
    git(&work, &["add", "a.txt"]);
    git(&work, &["commit", "-m", "one"]);
    let bare = base.join("origin.git");
    git(base, &["clone", "--bare", work.to_str().unwrap(), bare.to_str().unwrap()]);
    let p = bare.to_string_lossy().replace('\\', "/");
    if p.starts_with('/') {
        format!("file://{p}")
    } else {
        format!("file:///{p}")
    }
}

fn wait_clone(app: &mut Client, id: &str) -> Value {
    let end = Instant::now() + Duration::from_secs(60);
    loop {
        let s = app.call("project.clone_status", json!({"id": id})).unwrap();
        if s["state"] != "running" {
            return s;
        }
        assert!(Instant::now() < end, "clone did not finish: {s}");
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn code(e: &anyhow::Error) -> String {
    axelstra_core::msg::find(e).map(|c| c.code.clone()).unwrap_or_else(|| format!("{e:#}"))
}

#[test]
fn clone_registers_the_project_and_refuses_bad_input() {
    let (pipe, token) = start("clone");
    let mut app = Client::connect(&pipe, &token).unwrap();
    let base = tmp("clone");
    let url = bare_repo(&base);

    let dest = base.join("cloned");
    let id = app.call("project.clone", json!({"url": url, "dest": dest.to_str().unwrap(), "branch": "main", "depth": 1})).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let s = wait_clone(&mut app, &id);
    assert_eq!(s["state"], "done", "{s}");
    assert!(dest.join("a.txt").is_file());
    assert_eq!(s["project"]["name"], "cloned");
    let list = app.call("project.list", json!({})).unwrap();
    assert!(list.to_string().contains("cloned"), "{list}");

    // the folder now exists and is not empty
    let e = app.call("project.clone", json!({"url": url, "dest": dest.to_str().unwrap()})).unwrap_err();
    assert_eq!(code(&e), "project.clone_dest_exists");
    for (u, want) in
        [("", "project.clone_url_empty"), ("--upload-pack=x", "project.clone_url_invalid"), ("http://x/y", "project.clone_url_invalid")]
    {
        let e = app.call("project.clone", json!({"url": u, "dest": base.join("n").to_str().unwrap()})).unwrap_err();
        assert_eq!(code(&e), want, "{u}");
    }
    let e = app.call("project.clone", json!({"url": url, "dest": "relative/x"})).unwrap_err();
    assert_eq!(code(&e), "project.clone_dest_relative");
    let e = app.call("project.clone", json!({"url": url, "dest": base.join("d").to_str().unwrap(), "depth": 0})).unwrap_err();
    assert_eq!(code(&e), "project.clone_depth_invalid");
    let e = app.call("project.clone_status", json!({"id": "nope"})).unwrap_err();
    assert_eq!(code(&e), "project.clone_unknown");
}

#[test]
fn failed_clone_reports_a_code_and_removes_the_partial_folder() {
    let (pipe, token) = start("clonefail");
    let mut app = Client::connect(&pipe, &token).unwrap();
    let base = tmp("clonefail");
    let missing = base.join("nope.git").to_string_lossy().replace('\\', "/");
    let url = if missing.starts_with('/') { format!("file://{missing}") } else { format!("file:///{missing}") };
    let dest = base.join("out");
    let id = app.call("project.clone", json!({"url": url, "dest": dest.to_str().unwrap()})).unwrap()["id"].as_str().unwrap().to_string();
    let s = wait_clone(&mut app, &id);
    assert_eq!(s["state"], "failed", "{s}");
    assert_eq!(s["error_code"], "project.clone_failed");
    assert!(s["error_params"]["detail"].is_string());
    assert!(!dest.exists(), "partial folder left behind");
}

#[test]
fn cancel_stops_a_running_clone() {
    let (pipe, token) = start("cancel");
    let mut app = Client::connect(&pipe, &token).unwrap();
    let base = tmp("cancel");
    let url = bare_repo(&base);
    let dest = base.join("c");
    let id = app.call("project.clone", json!({"url": url, "dest": dest.to_str().unwrap()})).unwrap()["id"].as_str().unwrap().to_string();
    assert_eq!(app.call("project.clone_cancel", json!({"id": id})).unwrap()["ok"], true);
    // the clone of a tiny repo may win the race: then it is done and cancel changed nothing
    let s = wait_clone(&mut app, &id);
    assert!(s["state"] == "cancelled" || s["state"] == "done", "{s}");
    if s["state"] == "cancelled" {
        assert!(!dest.exists());
    }
}

#[test]
fn init_creates_the_repository_and_validates_the_name() {
    let (pipe, token) = start("init");
    let mut app = Client::connect(&pipe, &token).unwrap();
    let base = tmp("init");
    let p =
        app.call("project.init", json!({"parent": base.to_str().unwrap(), "name": "fresh", "readme": true, "gitignore": true})).unwrap();
    assert_eq!(p["name"], "fresh");
    let dir = base.join("fresh");
    assert!(dir.join(".git").is_dir() && dir.join("README.md").is_file() && dir.join(".gitignore").is_file());
    let e = app.call("project.init", json!({"parent": base.to_str().unwrap(), "name": "fresh"})).unwrap_err();
    assert_eq!(code(&e), "project.init_exists");
    for bad in ["", "..", "a/b", "con", "x:"] {
        let e = app.call("project.init", json!({"parent": base.to_str().unwrap(), "name": bad})).unwrap_err();
        assert_eq!(code(&e), "project.init_name_invalid", "{bad:?}");
    }
    let e = app.call("project.init", json!({"parent": base.join("missing").to_str().unwrap(), "name": "x"})).unwrap_err();
    assert_eq!(code(&e), "project.init_parent_missing");
    assert!(!base.join("..x").exists());
}

#[test]
fn add_discovered_takes_only_what_discover_offered() {
    let (pipe, token) = start("disc");
    let mut app = Client::connect(&pipe, &token).unwrap();
    let base = tmp("disc");
    let e = app.call("project.add_discovered", json!({"path": base.to_str().unwrap()})).unwrap_err();
    assert_eq!(code(&e), "project.not_discovered");
    // discover itself reads the real home folder: only its shape is checked
    let d = app.call("project.discover", json!({})).unwrap();
    assert!(d["items"].is_array());
    for i in d["items"].as_array().unwrap() {
        assert!(i["path"].is_string() && i["name"].is_string());
        assert!(i["source"] == "recent" || i["source"] == "scan");
    }
}

#[test]
fn host_info_lists_distros_and_ssh_hosts() {
    let (pipe, token) = start("hostinfo");
    let mut app = Client::connect(&pipe, &token).unwrap();
    let i = app.call("host.info", json!({})).unwrap();
    assert!(i["wsl_distros"].is_array() && i["ssh_hosts"].is_array(), "{i}");
}

#[test]
fn gh_repos_never_fails_without_gh() {
    let (pipe, token) = start("gh");
    let mut app = Client::connect(&pipe, &token).unwrap();
    let r = app.call("gh.repos", json!({"limit": 5})).unwrap();
    // gh may be missing, logged out, or (on a dev machine) really there; the answer always has a state
    assert!(["ok", "no_gh", "not_authed"].contains(&r["state"].as_str().unwrap()), "{r}");
    assert!(r["repos"].is_array());
}

#[test]
fn agents_may_not_use_the_sources() {
    let (pipe, token) = start("forbid");
    let mut app = Client::connect(&pipe, &token).unwrap();
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
    let mut agent = Client::connect_agent(&pipe, &"x".repeat(32), &tok).unwrap();
    for m in [
        "project.discover",
        "project.add_discovered",
        "project.clone",
        "project.clone_status",
        "project.clone_cancel",
        "project.init",
        "gh.repos",
    ] {
        let e = agent.call(m, json!({})).unwrap_err();
        assert_eq!(code(&e), "rpc.agent_forbidden", "{m}");
    }
}
