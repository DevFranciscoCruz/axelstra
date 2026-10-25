//! `axelstra --version` and `axelstra version`, run as the real binary with no daemon around (throwaway data dir).

use std::process::{Command, Output};

fn run(args: &[&str]) -> Output {
    let root = std::env::temp_dir().join(format!("axelstra-version-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    Command::new(env!("CARGO_BIN_EXE_axelstra"))
        .args(args)
        .env("AXELSTRA_DATA_DIR", root.join("data"))
        .env("AXELSTRA_TEST_PIPE", format!("ver{}", std::process::id()))
        .env_remove("AXELSTRA_HOST")
        .env_remove("AXELSTRA_AGENT_TOKEN")
        .output()
        .unwrap()
}

fn expected() -> String {
    format!(
        "axelstra {} ({}, built {}, PROTO {})",
        axelstra_core::buildinfo::VERSION,
        axelstra_core::buildinfo::COMMIT,
        axelstra_core::buildinfo::BUILT,
        axelstra_daemon::proto::PROTO
    )
}

#[test]
fn version_flags_print_one_line() {
    for flag in ["--version", "-V"] {
        let o = run(&[flag]);
        assert!(o.status.success(), "{flag}: {}", String::from_utf8_lossy(&o.stderr));
        assert_eq!(String::from_utf8_lossy(&o.stdout).trim(), expected());
    }
}

#[test]
fn version_json_has_the_fields_and_tolerates_a_missing_daemon() {
    let o = run(&["version", "--json"]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["version"], axelstra_core::buildinfo::VERSION);
    assert_eq!(v["commit"], axelstra_core::buildinfo::COMMIT);
    assert_eq!(v["built"], axelstra_core::buildinfo::BUILT);
    assert_eq!(v["proto"], axelstra_daemon::proto::PROTO);
    assert_eq!(v["security_min"], axelstra_daemon::proto::SECURITY_MIN);
    assert_eq!(v["build_id"], axelstra_daemon::proto::BUILD_ID);
    assert!(v["daemon"].is_null(), "no daemon runs on the test pipe: {v}");
}

#[test]
fn version_text_says_daemon_down() {
    let o = run(&["version"]);
    assert!(o.status.success());
    let out = String::from_utf8_lossy(&o.stdout);
    assert!(out.starts_with(&expected()), "{out}");
    assert!(out.contains("daemon: not running"), "{out}");
}
