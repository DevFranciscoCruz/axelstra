//! A coded error crosses the wire as `error` (English, for old clients and the CLI) plus `code` and `params`, and the
//! client rebuilds the code from it (`axelstra_core::msg::find` sees it through `client::Remote`).
mod common;
use axelstra_daemon::client::{Client, Remote};
use axelstra_daemon::server;
use serde_json::json;

#[test]
fn a_coded_error_keeps_its_code_and_params_through_the_wire() {
    let pipe = format!("{}-codes-{}", common::base_pipe(), std::process::id());
    let token = "c".repeat(32);
    let (p, t) = (pipe.clone(), token.clone());
    std::thread::spawn(move || {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap()
            .block_on(server::serve_with_store(p, t, common::store("codes")))
            .unwrap();
    });
    common::wait_ready(&pipe, &token);
    let mut c = Client::connect(&pipe, &token).unwrap();
    let e = c.call("git.changes", json!({"worktree": "nowhere/at/all"})).unwrap_err();
    assert!(e.downcast_ref::<Remote>().is_some(), "{e:#}");
    // the English text is still there
    assert!(format!("{e:#}").contains("not a worktree of a registered project"), "{e:#}");
    let coded = axelstra_core::msg::find(&e).expect("code survives the wire");
    assert_eq!(coded.code, "worktree.unknown");
    assert_eq!(coded.params["path"], "nowhere/at/all");
    // an unknown method is coded too, and its English text is what older callers look for
    let unknown = c.call("no.such.method", json!({})).unwrap_err();
    assert!(format!("{unknown:#}").contains("unknown method no.such.method"), "{unknown:#}");
    let coded = axelstra_core::msg::find(&unknown).expect("unknown method has a code");
    assert_eq!((coded.code.as_str(), coded.params["method"].as_str()), ("rpc.unknown_method", "no.such.method"));
}
