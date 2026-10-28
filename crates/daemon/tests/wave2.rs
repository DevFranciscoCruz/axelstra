//! The methods added by the wave-2 branches (PR events, budgets and rebase, workspace) all exist in one daemon, which
//! reports the merged PROTO. The agent-role refusals of the app-only ones live in `hardening.rs`.
mod common;
use axelstra_daemon::client::Client;
use axelstra_daemon::proto::PROTO;
use axelstra_daemon::server;
use serde_json::json;

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
fn one_daemon_serves_every_new_method_and_reports_proto_21() {
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
    common::wait_ready(&pipe, &token);
    let mut c = Client::connect(&pipe, &token).unwrap();
    assert_eq!(PROTO, 31, "wave 2 merged as 18; 19 added host.info, host.browse, hosts and agent-aware hooks.*; the stall watchdog (stalled, stall_minutes) makes 20; the watch-stream events that replace polling make 21; auto-integrate (worktree.integrate, project.integrate) makes 22; the Add Project sources (project.discover, clone, init, gh.repos) make 23; the full project removal (project.remove_preview, project.remove mode) makes 25 (24 is files.resolve_html); files.resolve_path and files.path_action make 26; the kanban.changed, orch.changed and automations.changed events make 27; clone.changed and docker.changed make 28; gh.pr_refresh makes 29; orch.cancel and integrate.undo make 30; session.hibernate and session.wake make 31");
    assert_eq!(c.proto, 30);
    // bogus arguments: any answer is fine except "unknown method" (the daemon lacks it) or an agent refusal
    for m in NEW_METHODS {
        if let Err(e) = c.call(m, json!({"project": "x", "worktree": "x", "path": "x"})) {
            let e = format!("{e:#}");
            assert!(!e.contains("unknown method"), "{m}: {e}");
            assert!(!e.contains("not allowed to agents"), "{m}: app role refused: {e}");
        }
    }
}
