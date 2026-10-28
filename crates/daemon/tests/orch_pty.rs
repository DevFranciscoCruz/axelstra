//! The orchestrator driving agents through daemon PTY sessions.

mod common;
use axelstra_core::git;
use axelstra_core::orch::{self, Options};
use axelstra_daemon::client::Client;
use axelstra_daemon::runner::DaemonRunner;
use axelstra_daemon::server;
use serde_json::json;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

fn sh(cwd: &Path, args: &[&str]) {
    assert!(Command::new("git").current_dir(cwd).args(args).status().unwrap().success(), "git {args:?}");
}

#[test]
fn agents_run_in_daemon_sessions_and_get_cleaned_up() {
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(120));
        eprintln!("watchdog: test stuck");
        std::process::exit(1);
    });

    let pipe = format!("{}-orch-{}", common::base_pipe(), std::process::id());
    let token = "k".repeat(32);
    let (p, t) = (pipe.clone(), token.clone());
    std::thread::spawn(move || {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap()
            .block_on(server::serve_with_store(p, t, common::store("srv")))
            .unwrap();
    });
    common::wait_ready(&pipe, &token);

    let root = std::env::temp_dir().join(format!("axelstra-pty-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let repo = root.join("repo");
    std::fs::create_dir_all(repo.join(".kanban")).unwrap();
    sh(&repo, &["init", "-q", "-b", "main"]);
    sh(&repo, &["config", "user.email", "t@t"]);
    sh(&repo, &["config", "user.name", "t"]);
    std::fs::write(repo.join(".gitignore"), ".kanban/\n").unwrap();
    std::fs::write(repo.join("shared.txt"), "base\n").unwrap();
    sh(&repo, &["add", "-A"]);
    sh(&repo, &["commit", "-qm", "init"]);
    let repo = git::canon(&repo).unwrap();
    for (id, deps) in [("a", ""), ("b", "a"), ("c", "")] {
        std::fs::write(
            repo.join(".kanban").join(format!("{id}.md")),
            format!("---\nid: {id}\nstatus: todo\ndepends_on: [{deps}]\n---\ntask {id}\n"),
        )
        .unwrap();
    }

    let tools = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools");
    let agent: Vec<String> = if cfg!(windows) {
        let script = git::canon(&tools.join("fake-agent.ps1")).unwrap();
        ["powershell", "-NoProfile", "-File", &script.to_string_lossy()].map(String::from).to_vec()
    } else {
        let script = git::canon(&tools.join("fake-agent.sh")).unwrap();
        ["sh", &script.to_string_lossy()].map(String::from).to_vec()
    };
    let report = orch::run(&Options {
        repo: repo.clone(),
        name: "t".into(),
        max_concurrency: 2,
        wt_root: root.join("wt"),
        agent_override: Some(agent),
        poll_ms: 3_600_000, // an hour: the run advances on exit/result events, not on the backoff
        cancel: None,
        runner: Some(Arc::new(DaemonRunner::connect(&pipe, &token).unwrap())),
    })
    .unwrap();

    assert_eq!(report.merged, ["a", "b", "c"], "failed: {:?}", report.failed);
    for f in ["a.txt", "b.txt", "c.txt"] {
        assert!(report.integ_wt.join(f).exists(), "{f} missing");
    }

    // every agent session was removed from the daemon once its task finished
    let mut c = Client::connect(&pipe, &token).unwrap();
    assert_eq!(c.call("session.list", json!({})).unwrap(), json!([]));
}
