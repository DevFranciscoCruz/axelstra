//! What the app needs to tell a stale daemon from its own build: `hello` carries version, commit, build date and exe,
//! the client keeps them, and `Client::server_exe` learns the daemon's executable from the OS (the pipe's server
//! process on Windows, the socket peer on Linux and macOS). The daemon here runs in this test process on a temp pipe.

mod common;
use axelstra_daemon::client::Client;
use axelstra_daemon::server;

#[test]
fn hello_reports_the_build_and_the_client_finds_the_exe() {
    let pipe = common::pipe("build");
    let token = "t".repeat(32);
    let (p, t) = (pipe.clone(), token.clone());
    std::thread::spawn(move || {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap()
            .block_on(server::serve_with_store(p, t, common::store("build")))
            .unwrap();
    });
    common::wait_ready(&pipe, &token);
    let c = Client::connect(&pipe, &token).unwrap();
    assert_eq!(c.version, axelstra_core::buildinfo::VERSION);
    assert_eq!(c.commit, axelstra_core::buildinfo::COMMIT);
    assert_eq!(c.built, axelstra_core::buildinfo::BUILT);
    assert_eq!(c.hello_result()["exe"].as_str().map(std::path::PathBuf::from), std::env::current_exe().ok());
    // the daemon is this very process, so the pipe's server is this test binary
    let canon = |p: std::path::PathBuf| std::fs::canonicalize(&p).unwrap_or(p);
    let exe = c.server_exe().map(canon);
    assert_eq!(exe, std::env::current_exe().ok().map(canon));
    // a clone keeps what hello said
    let d = c.try_clone().unwrap();
    assert_eq!((d.version, d.commit, d.built), (c.version.clone(), c.commit.clone(), c.built.clone()));
}
