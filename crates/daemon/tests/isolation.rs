//! The guards behind "tests never use the real data dir" (docs/TESTING.md).
mod common;
use axelstra_daemon::{proto, server};

#[test]
fn serve_refuses_the_real_data_dir_and_pipe_under_test() {
    let tmp = common::data_dir(); // sets AXELSTRA_TESTING=1 and AXELSTRA_DATA_DIR
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let real = proto::default_data_dir().unwrap();
    assert_ne!(real, tmp);
    // the real store: refused before anything is created or written
    let e = rt.block_on(server::serve_with_store("axl-never-listens".into(), "c".repeat(32), real.join("projects.json"))).unwrap_err();
    assert!(e.to_string().contains("refusing"), "{e}");
    // the real pipe with an isolated store: refused too
    let e = rt.block_on(server::serve_with_store(proto::default_pipe_name(), "c".repeat(32), tmp.join("projects.json"))).unwrap_err();
    assert!(e.to_string().contains("refusing"), "{e}");
}

#[test]
fn the_override_moves_the_data_dir_and_the_pipe() {
    let tmp = common::data_dir();
    assert_eq!(proto::data_dir(), tmp);
    assert_eq!(proto::token_path(), tmp.join("token"));
    assert_ne!(proto::pipe_name(), proto::default_pipe_name());
}
