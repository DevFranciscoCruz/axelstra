//! `gh.send_comment` needs a live lead in the worktree before it asks GitHub for anything, and `git.suggest_pr` refuses
//! a base that git could read as an option.
use axelstra_daemon::client::Client;
use axelstra_daemon::server;
use serde_json::{json, Value};
use std::path::Path;
use std::process::Command;
use std::time::Duration;

fn git(cwd: &Path, args: &[&str]) {
    let o = Command::new("git").current_dir(cwd).args(args).output().unwrap();
    assert!(o.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&o.stderr));
}

fn shell() -> (&'static str, Vec<&'static str>) {
    if cfg!(windows) {
        ("cmd", vec!["/q"])
    } else {
        ("sh", vec![])
    }
}

fn spawn(c: &mut Client, project: &str, wt: &Value, extra: Value) -> u64 {
    let (prog, args) = shell();
    let mut p = json!({"program": prog, "args": args, "label": "claude", "meta": {"project": project, "worktree": wt, "kind": "agent", "agent": "claude"}});
    for (k, v) in extra.as_object().unwrap() {
        p[k] = v.clone();
    }
    c.call("session.spawn", p).unwrap()["id"].as_u64().unwrap()
}

#[test]
fn sending_a_comment_needs_a_live_lead_and_a_known_worktree() {
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(90));
        eprintln!("watchdog: test stuck");
        std::process::exit(1);
    });
    let root = std::env::temp_dir().join(format!("axelstra-prc-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let repo = root.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["config", "user.email", "me@example.com"]);
    git(&repo, &["config", "user.name", "Me"]);
    std::fs::write(repo.join("a.txt"), "x").unwrap();
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-qm", "init"]);

    let pipe = format!("{}-prc-{}", axelstra_daemon::proto::pipe_name(), std::process::id());
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
    std::thread::sleep(Duration::from_millis(300));
    let mut c = Client::connect(&pipe, &token).unwrap();
    c.call("project.add", json!({"path": &repo})).unwrap();
    let list = c.call("project.list", json!({})).unwrap();
    let (project, wt) = (list[0]["id"].as_str().unwrap().to_string(), list[0]["worktrees"][0]["path"].clone());
    let send = |c: &mut Client, wt: &Value| {
        c.call("gh.send_comment", json!({"worktree": wt, "project": project, "number": 7, "comment_id": 11, "dry_run": false}))
            .unwrap_err()
            .to_string()
    };

    // nobody works here yet
    assert!(send(&mut c, &wt).contains("no live lead"), "no session at all");
    // a path that is not a worktree of a project never gets as far as a lookup
    let e = send(&mut c, &json!("C:/not/a/worktree"));
    assert!(e.contains("not a worktree of a registered project"), "{e}");
    // missing ids are refused as such
    let e = c.call("gh.send_comment", json!({"worktree": wt, "project": project})).unwrap_err().to_string();
    assert!(e.contains("missing number"), "{e}");

    // with a lead the request goes on to GitHub (no remote here, so gh fails): the lead check is not what stops it
    spawn(&mut c, &project, &wt, json!({}));
    let lead = c.call("session.list", json!({})).unwrap().as_array().unwrap().iter().any(|s| s["meta"]["lead"] == true);
    assert!(lead, "the first agent of a worktree leads");
    let e = send(&mut c, &wt);
    assert!(!e.contains("no live lead"), "{e}");

    // a dry run without a lead also fails only at GitHub; and the PR suggestion refuses an option as base
    let e = c.call("git.suggest_pr", json!({"worktree": wt, "base": "--output=x"})).unwrap_err().to_string();
    assert!(e.contains("invalid base ref"), "{e}");
}
