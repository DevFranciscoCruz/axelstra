//! A daemon session tells the orchestrator when it ends: no polling backoff.

mod common;
use axelstra_core::runner::{AgentSpawn, Runner};
use axelstra_core::wake::Wake;
use axelstra_daemon::runner::DaemonRunner;
use axelstra_daemon::server;
use std::time::{Duration, Instant};

#[test]
fn a_session_exit_bumps_the_wake_without_polling() {
    let pipe = common::pipe("runner-exit");
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
    let runner = DaemonRunner::connect(&pipe, &token).unwrap();
    let (program, args): (&str, &[&str]) =
        if cfg!(windows) { ("cmd", &["/C", "ping -n 3 127.0.0.1 >nul"]) } else { ("sh", &["-c", "sleep 1"]) };
    let mut h = runner
        .spawn(&AgentSpawn {
            program: program.into(),
            args: args.iter().map(|s| s.to_string()).collect(),
            cwd: common::data_dir(),
            env: vec![],
            label: "t".into(),
            log: None,
        })
        .unwrap();
    let wake = Wake::new();
    let seen = wake.now();
    assert!(h.on_exit(&wake), "the handle must push exits");
    let t0 = Instant::now();
    // the bump comes from the exit event, long before this limit
    assert_ne!(wake.wait(seen, Some(Duration::from_secs(20))), seen, "no bump");
    assert!(t0.elapsed() < Duration::from_secs(15));
    assert!(h.try_wait().unwrap().is_some());
}
