//! `worktree.sweep {artifacts}` reclaims the ignored build output of an inactive worktree (cargo's `target/` next to
//! `Cargo.toml`) and never: a tracked file, what a symlink/junction points to, a linked candidate, a worktree in use
//! or used recently, or the project's main checkout. `disk.status` reports the sizes.
mod common;
use axelstra_daemon::client::Client;
use axelstra_daemon::server;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Command;

fn sh(cwd: &Path, args: &[&str]) {
    assert!(Command::new("git").current_dir(cwd).args(args).status().unwrap().success(), "git {args:?}");
}

fn git_out(cwd: &Path, args: &[&str]) -> String {
    String::from_utf8_lossy(&Command::new("git").current_dir(cwd).args(args).output().unwrap().stdout).trim().to_string()
}

fn repo(root: &Path) -> PathBuf {
    let r = root.join("proj");
    std::fs::create_dir_all(&r).unwrap();
    sh(&r, &["init", "-q", "-b", "main"]);
    sh(&r, &["config", "user.email", "t@t"]);
    sh(&r, &["config", "user.name", "t"]);
    std::fs::write(r.join("Cargo.toml"), "[workspace]\n").unwrap();
    std::fs::write(r.join("package.json"), "{}").unwrap();
    std::fs::write(r.join(".gitignore"), "target/\nnode_modules/\nbuild/\n").unwrap();
    // an ignored folder that still holds a tracked file: never a candidate
    std::fs::create_dir_all(r.join("build")).unwrap();
    std::fs::write(r.join("build/tracked.txt"), "tracked").unwrap();
    sh(&r, &["add", "Cargo.toml", "package.json", ".gitignore"]);
    sh(&r, &["add", "-f", "build/tracked.txt"]);
    sh(&r, &["commit", "-qm", "init"]);
    r
}

fn start(tag: &str) -> (Client, PathBuf) {
    let store = common::store(tag);
    let root = store.parent().unwrap().to_path_buf();
    let pipe = common::pipe(tag);
    let token = "a".repeat(32);
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

/// Some build output in `target/` (4 KB).
fn build(dir: &Path) {
    std::fs::create_dir_all(dir.join("target/debug/deps")).unwrap();
    std::fs::write(dir.join("target/debug/deps/a.rlib"), vec![7u8; 4096]).unwrap();
}

/// A link to a folder: a junction on Windows (no privilege needed), else a symlink. False when it cannot be made.
fn link(target: &Path, at: &Path) -> bool {
    #[cfg(windows)]
    return Command::new("cmd").args(["/C", "mklink", "/J"]).arg(at).arg(target).output().is_ok_and(|o| o.status.success());
    #[cfg(not(windows))]
    return std::os::unix::fs::symlink(target, at).is_ok();
}

/// Makes the worktree look unused for two days: the mtimes of its git HEAD, index and logs/HEAD.
fn age(wt: &Path) {
    let gitdir = PathBuf::from(git_out(wt, &["rev-parse", "--absolute-git-dir"]));
    let old = std::time::SystemTime::now() - std::time::Duration::from_secs(2 * 24 * 3600);
    for f in ["HEAD", "index", "logs/HEAD"] {
        if let Ok(h) = std::fs::OpenOptions::new().write(true).open(gitdir.join(f)) {
            h.set_modified(old).unwrap();
        }
    }
}

/// The reason given for `path`; the daemon reports git's canonical spelling, so `path` is compared canonical too.
fn code_of(v: &Value, path: &str) -> Option<String> {
    let canon = axelstra_core::git::canon(Path::new(path)).map(|c| c.to_string_lossy().to_string());
    let path = canon.as_deref().unwrap_or(path);
    v["skipped"]
        .as_array()?
        .iter()
        .find(|s| norm(s["path"].as_str().unwrap_or_default()) == norm(path))
        .map(|s| s["code"].as_str().unwrap_or_default().to_string())
}

fn norm(p: &str) -> String {
    axelstra_core::paths::norm(p)
}

#[test]
fn only_the_ignored_build_output_of_an_inactive_worktree_goes() {
    let (mut c, root) = start("art");
    let r = repo(&root);
    // the automatic pass stays out of the way: this test drives the sweep itself
    c.call("settings.set", json!({"disk": {"enabled": false}, "janitor": {"artifact_quiet_hours": 1}})).unwrap();
    let id = c.call("project.add", json!({"path": &r})).unwrap()["id"].as_str().unwrap().to_string();
    let mk = |c: &mut Client, name: &str| {
        PathBuf::from(c.call("worktree.create", json!({"project": id, "name": name})).unwrap()["path"].as_str().unwrap())
    };
    let (old, recent, busy) = (mk(&mut c, "old"), mk(&mut c, "recent"), mk(&mut c, "busy"));
    for d in [&r, &old, &recent, &busy] {
        build(d);
    }
    // a link inside target/ to a folder outside: unlinked, its target stays whole
    let outside = root.join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("keep.txt"), "keep").unwrap();
    let linked = link(&outside, &old.join("target/debug/out"));
    // node_modules that is itself a link: never a candidate
    let shared = root.join("shared_modules");
    std::fs::create_dir_all(&shared).unwrap();
    std::fs::write(shared.join("pkg.js"), "x").unwrap();
    let nm_linked = link(&shared, &old.join("node_modules"));
    age(&old);
    age(&busy);
    let (prog, args) = if cfg!(windows) { ("cmd", vec!["/q"]) } else { ("sh", vec![]) };
    let meta = json!({"project": id, "worktree": busy.to_string_lossy(), "kind": "terminal"});
    c.call("session.spawn", json!({"program": prog, "args": args, "cwd": &busy, "meta": meta})).unwrap();

    // dry run: says what would go, removes nothing
    let dry = c.call("worktree.sweep", json!({"artifacts": true, "dry_run": true})).unwrap();
    assert!(old.join("target").exists(), "{dry}");
    let row =
        dry["artifacts"].as_array().unwrap().iter().find(|a| norm(a["path"].as_str().unwrap()) == norm(&old.to_string_lossy())).cloned();
    assert!(row.is_some(), "{dry}");

    let v = c.call("worktree.sweep", json!({"artifacts": true})).unwrap();
    let got: Vec<String> = v["artifacts"].as_array().unwrap().iter().map(|a| norm(a["path"].as_str().unwrap())).collect();
    assert_eq!(got, vec![norm(&old.to_string_lossy())], "{v}");
    let dirs = v["artifacts"][0]["dirs"].as_array().unwrap();
    assert_eq!(dirs.len(), 1, "only target/: {v}");
    assert_eq!(dirs[0]["kind"], "target");
    assert!(!old.join("target").exists(), "{v}");
    // tracked file in an ignored folder, the link's target, a linked node_modules: all untouched
    assert_eq!(std::fs::read_to_string(old.join("build/tracked.txt")).unwrap(), "tracked");
    assert_eq!(std::fs::read_to_string(outside.join("keep.txt")).unwrap(), "keep", "linked={linked}");
    if nm_linked {
        assert!(old.join("node_modules").exists());
        assert_eq!(std::fs::read_to_string(shared.join("pkg.js")).unwrap(), "x");
    }
    // recent and busy are skipped with their reason; the main checkout is never in a pass
    assert_eq!(code_of(&v, &recent.to_string_lossy()).as_deref(), Some("artifacts.recent"), "{v}");
    assert_eq!(code_of(&v, &busy.to_string_lossy()).as_deref(), Some("artifacts.busy"), "{v}");
    assert!(recent.join("target").exists() && busy.join("target").exists() && r.join("target").exists());

    // the user's force passes the quiet rule only: never a session, never the main checkout
    let f = c.call("worktree.sweep", json!({"artifacts": true, "force": true, "path": &busy})).unwrap();
    assert_eq!(code_of(&f, &busy.to_string_lossy()).as_deref(), Some("artifacts.busy"), "{f}");
    let m = c.call("worktree.sweep", json!({"artifacts": true, "force": true, "path": &r})).unwrap();
    assert_eq!(code_of(&m, &r.to_string_lossy()).as_deref(), Some("artifacts.main"), "{m}");
    assert!(r.join("target/debug/deps/a.rlib").exists() && busy.join("target").exists());
    let fr = c.call("worktree.sweep", json!({"artifacts": true, "force": true, "path": &recent})).unwrap();
    assert!(fr["freed_mb"].is_u64() && !recent.join("target").exists(), "{fr}");

    // the status reports the volume and the worktrees with their sizes
    let s = c.call("disk.status", json!({"refresh": true})).unwrap();
    assert!(!s["volumes"].as_array().unwrap().is_empty(), "{s}");
    let rows = s["worktrees"].as_array().unwrap();
    let b = rows.iter().find(|w| norm(w["path"].as_str().unwrap()) == norm(&busy.to_string_lossy())).unwrap();
    assert_eq!(b["active"], true, "{s}");
    assert_eq!(b["reclaimable_mb"], 0, "{s}");
    assert!(!b["sessions"].as_array().unwrap().is_empty(), "{s}");
    assert!(rows.iter().all(|w| norm(w["path"].as_str().unwrap()) != norm(&r.to_string_lossy())), "main is not listed: {s}");
    let e = c.call("disk.ensure", json!({})).unwrap();
    assert_eq!(e["low"], false, "disk off: {e}");
}
