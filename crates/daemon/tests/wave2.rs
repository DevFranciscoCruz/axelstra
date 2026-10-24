//! The methods added by the wave-2 branches (PR events, budgets and rebase, workspace) all exist in one daemon, which
//! reports the merged PROTO. The agent-role refusals of the app-only ones live in `hardening.rs`.
mod common;
use axelstra_daemon::client::Client;
use axelstra_daemon::proto::PROTO;
use axelstra_daemon::server;
use serde_json::json;
use std::time::Duration;

const NEW_METHODS: &[&str] = &[
    "git.suggest_pr",
    "gh.comments",
    "gh.send_comment",
    "triggers.get",
    "triggers.preview",
    "triggers.set",
    "triggers.events",
    "budget.status",
    "budget.check",
    "budget.events",
    "dispatch.overlap_files",
    "worktree.rebase",
    "git.timeline",
    "workspace.export",
    "workspace.import_preview",
    "workspace.import",
];

#[test]
fn one_daemon_serves_every_new_method_and_reports_proto_18() {
    let pipe = format!("{}-wave2-{}", common::base_pipe(), std::process::id());
    let token = "d".repeat(32);
    let (p, t) = (pipe.clone(), token.clone());
    std::thread::spawn(move || {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap()
            .block_on(server::serve_with_store(p, t, common::store("wave2")))
            .unwrap();
    });
    std::thread::sleep(Duration::from_millis(300));
    let mut c = Client::connect(&pipe, &token).unwrap();
    assert_eq!(PROTO, 18, "three branches claimed 17; the merged contract is 18");
    assert_eq!(c.proto, 18);
    // bogus arguments: any answer is fine except "unknown method" (the daemon lacks it) or an agent refusal
    for m in NEW_METHODS {
        if let Err(e) = c.call(m, json!({"project": "x", "worktree": "x", "path": "x"})) {
            let e = format!("{e:#}");
            assert!(!e.contains("unknown method"), "{m}: {e}");
            assert!(!e.contains("not allowed to agents"), "{m}: app role refused: {e}");
        }
    }
}
