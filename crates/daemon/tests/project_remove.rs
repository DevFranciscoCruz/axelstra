//! Removing a project with `mode: "all"` closes its sessions, removes its worktrees and `ax/*` branches and its records,
//! and never touches the project's own checkout or another project.
mod common;
use axelstra_daemon::client::Client;
use axelstra_daemon::server;
use serde_json::{json, Value};
use std::path::Path;
use std::process::Command;

fn sh(cwd: &Path, args: &[&str]) {
    assert!(Command::new("git").current_dir(cwd).args(args).status().unwrap().success(), "git {args:?}");
}

fn repo(root: &Path, name: &str) -> std::path::PathBuf {
    let r = root.join(name);
    std::fs::create_dir_all(&r).unwrap();
    sh(&r, &["init", "-q", "-b", "main"]);
    sh(&r, &["config", "user.email", "t@t"]);
    sh(&r, &["config", "user.name", "t"]);
    std::fs::write(r.join("a.txt"), "x").unwrap();
    sh(&r, &["add", "a.txt"]);
    sh(&r, &["commit", "-qm", "init"]);
    r
}

fn start(tag: &str) -> (Client, std::path::PathBuf) {
    let store = common::store(tag);
    let root = store.parent().unwrap().to_path_buf();
    let pipe = common::pipe(tag);
    let token = "r".repeat(32);
    let (p, t, s) = (pipe.clone(), token.clone(), store.clone());
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
    (Client::connect(&pipe, &token).unwrap(), root)
}

fn terminal(c: &mut Client, project: &str, wt: &str) -> u64 {
    let (prog, args) = if cfg!(windows) { ("cmd", vec!["/q"]) } else { ("sh", vec![]) };
    let meta = json!({"project": project, "worktree": wt, "kind": "terminal"});
    c.call("session.spawn", json!({"program": prog, "args": args, "cwd": wt, "meta": meta})).unwrap()["id"].as_u64().unwrap()
}

fn branches(repo: &Path) -> String {
    String::from_utf8_lossy(&Command::new("git").current_dir(repo).args(["branch", "--format=%(refname:short)"]).output().unwrap().stdout)
        .to_string()
}

fn path(v: &Value) -> String {
    v["path"].as_str().unwrap().to_string()
}

#[test]
fn removing_a_project_cleans_everything_and_leaves_the_checkout_and_other_projects() {
    let (mut c, root) = start("rmall");
    let (a, b) = (repo(&root, "alpha"), repo(&root, "beta"));
    let aid = c.call("project.add", json!({"path": &a})).unwrap()["id"].as_str().unwrap().to_string();
    let bid = c.call("project.add", json!({"path": &b})).unwrap()["id"].as_str().unwrap().to_string();
    // alpha: one dirty worktree, one with a commit nobody integrated, one clean
    let dirty = c.call("worktree.create", json!({"project": aid, "name": "dirty"})).unwrap();
    let open = c.call("worktree.create", json!({"project": aid, "name": "open"})).unwrap();
    let clean = c.call("worktree.create", json!({"project": aid, "name": "clean"})).unwrap();
    let keep = c.call("worktree.create", json!({"project": bid, "name": "keep"})).unwrap();
    std::fs::write(Path::new(&path(&dirty)).join("wip.txt"), "w").unwrap();
    std::fs::write(Path::new(&path(&open)).join("o.txt"), "o").unwrap();
    sh(Path::new(&path(&open)), &["add", "o.txt"]);
    sh(Path::new(&path(&open)), &["commit", "-qm", "unmerged"]);
    // sessions in alpha (main checkout and a worktree) and one in beta; records of both projects on disk
    terminal(&mut c, &aid, &a.to_string_lossy());
    terminal(&mut c, &aid, &path(&dirty));
    let other = terminal(&mut c, &bid, &path(&keep));
    for id in [&aid, &bid] {
        std::fs::create_dir_all(root.join("notes").join(id)).unwrap();
        std::fs::write(root.join("notes").join(id).join("Tarefa 1.md"), "# t").unwrap();
    }

    let pv = c.call("project.remove_preview", json!({"id": aid})).unwrap();
    assert_eq!(pv["terminals"], 2, "{pv}");
    assert_eq!(pv["sessions"], 0, "{pv}");
    let rows = pv["worktrees"].as_array().unwrap();
    assert_eq!(rows.len(), 3, "{pv}");
    let flags = |w: &Value| {
        let r = rows.iter().find(|r| r["path"] == path(w).as_str()).unwrap();
        (r["dirty"].as_bool().unwrap(), r["unmerged"].as_bool().unwrap())
    };
    assert_eq!(flags(&dirty), (true, false));
    assert_eq!(flags(&open), (false, true));
    assert_eq!(flags(&clean), (false, false));
    // read only
    assert!(Path::new(&path(&dirty)).exists() && c.call("project.list", json!({})).unwrap().as_array().unwrap().len() == 2);

    let bad = c.call("project.remove", json!({"id": aid, "mode": "nope"})).unwrap_err();
    assert_eq!(axelstra_core::msg::find(&bad).unwrap().code, "project.remove_bad_mode");

    let r = c.call("project.remove", json!({"id": aid, "mode": "all"})).unwrap();
    assert_eq!(r["removed"], true, "{r}");
    assert_eq!(r["sessions_closed"], 2, "{r}");
    let (removed, pending) = (r["worktrees_removed"].as_u64().unwrap(), r["worktrees_pending"].as_u64().unwrap());
    assert_eq!(removed + pending, 3, "{r}");
    if pending == 0 {
        assert_eq!(r["branches_deleted"], 3, "{r}");
        for w in [&dirty, &open, &clean] {
            assert!(!Path::new(&path(w)).exists(), "{} must be gone", path(w));
        }
        let left = branches(&a);
        assert!(!left.contains("ax/"), "{left}");
    }
    // never the checkout, never another project
    assert!(a.join("a.txt").exists() && a.join(".git").exists());
    assert!(Path::new(&path(&keep)).exists());
    assert!(branches(&b).contains("ax/keep"));
    assert!(!root.join("notes").join(&aid).exists());
    assert!(root.join("notes").join(&bid).exists());
    let list = c.call("project.list", json!({})).unwrap();
    let ids: Vec<&str> = list.as_array().unwrap().iter().map(|p| p["id"].as_str().unwrap()).collect();
    assert_eq!(ids, [bid.as_str()]);
    let sessions = c.call("session.list", json!({})).unwrap();
    let live: Vec<u64> = sessions.as_array().unwrap().iter().map(|s| s["id"].as_u64().unwrap()).collect();
    assert_eq!(live, [other], "only beta's terminal is left");
}

#[test]
fn list_mode_and_the_old_call_only_drop_the_project_from_the_list() {
    let (mut c, root) = start("rmlist");
    let a = repo(&root, "gamma");
    let id = c.call("project.add", json!({"path": &a})).unwrap()["id"].as_str().unwrap().to_string();
    let wt = c.call("worktree.create", json!({"project": id, "name": "stay"})).unwrap();
    let t = terminal(&mut c, &id, &path(&wt));
    let r = c.call("project.remove", json!({"id": id})).unwrap();
    assert_eq!((r["removed"].clone(), r["sessions_closed"].clone(), r["worktrees_removed"].clone()), (json!(true), json!(0), json!(0)));
    assert_eq!(c.call("project.list", json!({})).unwrap(), json!([]));
    assert!(Path::new(&path(&wt)).exists());
    assert!(branches(&a).contains("ax/stay"));
    assert!(c.call("session.list", json!({})).unwrap().as_array().unwrap().iter().any(|s| s["id"] == t));
    assert!(c.call("project.remove_preview", json!({"id": id})).is_err(), "an unknown project");
}

/// A process outside our sessions holds a worktree: it stays whole and queued, then goes once the holder exits.
#[test]
fn a_held_worktree_is_queued_and_removed_once_released() {
    let (mut c, root) = start("rmheld");
    let a = repo(&root, "delta");
    let id = c.call("project.add", json!({"path": &a})).unwrap()["id"].as_str().unwrap().to_string();
    let wt = c.call("worktree.create", json!({"project": id, "name": "held"})).unwrap();
    let wp = path(&wt);
    let (prog, args): (&str, &[&str]) = if cfg!(windows) { ("ping", &["-n", "60", "127.0.0.1"]) } else { ("sleep", &["60"]) };
    let mut holder = Command::new(prog)
        .args(args)
        .current_dir(&wp)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    // some Windows builds let a folder move while a process has it as cwd: then nothing is held and nothing is pending
    let probe = format!("{wp}-probe");
    let held = std::fs::rename(&wp, &probe).is_err();
    if !held {
        std::fs::rename(&probe, &wp).unwrap();
    }
    let r = c.call("project.remove", json!({"id": id, "mode": "all"})).unwrap();
    if held {
        assert_eq!(r["worktrees_pending"], 1, "{r}");
        assert!(Path::new(&wp).exists() && branches(&a).contains("ax/held"), "a held worktree is left whole");
    }
    let _ = holder.kill();
    let _ = holder.wait();
    // the clean-up (and the janitor, woken by exits and its backoff) tries the queue again
    c.call("worktree.sweep", json!({})).unwrap();
    assert!(!Path::new(&wp).exists());
    assert!(!branches(&a).contains("ax/held"));
    assert!(a.join("a.txt").exists());
}
