#![cfg(windows)]

use axelstra_daemon::client::Client;
use axelstra_daemon::server;
use serde_json::{json, Value};
use std::path::Path;
use std::process::Command;
use std::time::Duration;

fn sh(cwd: &Path, args: &[&str]) {
    assert!(Command::new("git").current_dir(cwd).args(args).status().unwrap().success(), "git {args:?}");
}

fn file<'a>(changes: &'a Value, path: &str) -> &'a Value {
    changes["files"].as_array().unwrap().iter().find(|f| f["path"] == path).unwrap_or_else(|| panic!("{path} not listed: {changes}"))
}

#[test]
fn changes_and_diff() {
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(90));
        eprintln!("watchdog: test stuck");
        std::process::exit(1);
    });

    let root = std::env::temp_dir().join(format!("axelstra-chg-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let repo = root.join("r");
    std::fs::create_dir_all(&repo).unwrap();
    sh(&repo, &["init", "-q", "-b", "main"]);
    sh(&repo, &["config", "user.email", "t@t"]);
    sh(&repo, &["config", "user.name", "t"]);
    std::fs::write(repo.join("keep.txt"), "one\ntwo\nthree\n").unwrap();
    std::fs::write(repo.join("gone.txt"), "bye\n").unwrap();
    sh(&repo, &["add", "-A"]);
    sh(&repo, &["commit", "-qm", "init"]);

    let pipe = format!("{}-chg-{}", axelstra_daemon::proto::pipe_name(), std::process::id());
    let token = "c".repeat(32);
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

    let pr = c.call("project.add", json!({"path": &repo})).unwrap();
    let wt = c.call("worktree.create", json!({"project": pr["id"], "name": "w"})).unwrap();
    let wtp = wt["path"].as_str().unwrap().to_string();
    let wtp = Path::new(&wtp);

    // a commit on the branch, then uncommitted edits on top
    std::fs::write(wtp.join("committed.txt"), "c1\nc2\n").unwrap();
    sh(wtp, &["add", "-A"]);
    sh(wtp, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qm", "on branch"]);
    std::fs::write(wtp.join("keep.txt"), "one\nTWO\nthree\nfour\n").unwrap();
    std::fs::remove_file(wtp.join("gone.txt")).unwrap();
    std::fs::write(wtp.join("new.txt"), "n1\nn2\nn3\n").unwrap();
    let w = wt["path"].clone();

    // vs HEAD: only uncommitted work
    let ch = c.call("git.changes", json!({"worktree": w})).unwrap();
    let keep = file(&ch, "keep.txt");
    assert_eq!((keep["added"].as_u64(), keep["removed"].as_u64()), (Some(2), Some(1)));
    assert_eq!(file(&ch, "gone.txt")["status"], "D");
    let new = file(&ch, "new.txt");
    assert_eq!((new["status"].as_str(), new["untracked"].as_bool(), new["added"].as_u64()), (Some("A"), Some(true), Some(3)));
    assert!(ch["files"].as_array().unwrap().iter().all(|f| f["path"] != "committed.txt"));

    // vs main: includes what was committed on the branch
    let br = c.call("git.changes", json!({"worktree": w, "base": "main"})).unwrap();
    assert_eq!(file(&br, "committed.txt")["added"], 2);
    assert_eq!(file(&br, "keep.txt")["added"], 2);

    // diffs
    let d = c.call("git.diff", json!({"worktree": w, "path": "keep.txt"})).unwrap()["diff"].as_str().unwrap().to_string();
    assert!(d.contains("-two") && d.contains("+TWO") && d.contains("+four"), "{d}");
    let d = c.call("git.diff", json!({"worktree": w, "path": "new.txt"})).unwrap()["diff"].as_str().unwrap().to_string();
    assert!(d.contains("@@ -0,0 +1,3 @@") && d.contains("+n2"), "{d}");

    // safety: only registered worktrees, only paths inside them
    assert!(c.call("git.changes", json!({"worktree": root})).is_err());
    assert!(c.call("git.diff", json!({"worktree": w, "path": "..\\r\\keep.txt"})).is_err());
    assert!(c.call("git.diff", json!({"worktree": w, "path": "C:\\Windows\\win.ini"})).is_err());

    // review: commits the branch has over main, and one commit's patch
    let log = c.call("git.log", json!({"worktree": w, "base": "main"})).unwrap();
    let commits = log["commits"].as_array().unwrap();
    assert_eq!(commits.len(), 1, "{log}");
    assert_eq!(commits[0]["subject"], "on branch");
    assert_eq!((commits[0]["files"].as_u64(), commits[0]["added"].as_u64()), (Some(1), Some(2)));
    let sha = commits[0]["sha"].as_str().unwrap();
    let p = c.call("git.show", json!({"worktree": w, "sha": sha})).unwrap()["diff"].as_str().unwrap().to_string();
    assert!(p.contains("+c1") && p.contains("committed.txt"), "{p}");
    assert!(c.call("git.show", json!({"worktree": w, "sha": "--output=x"})).is_err(), "option injection");
    assert!(c.call("git.show", json!({"worktree": w, "sha": "zzzzzzzz"})).is_err());
}
