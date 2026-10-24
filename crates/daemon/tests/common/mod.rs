//! Test isolation: a daemon test never touches the user's real data dir or pipe.
//!
//! The first call in a test binary makes one temp data dir and sets `AXELSTRA_DATA_DIR` and `AXELSTRA_TESTING=1` for the
//! whole process, so everything that reads `proto::data_dir()` (lag.log, token, locks) lands there, and the Windows
//! pipe name derives from it. `server::serve*` also refuses the real dir while testing. See docs/TESTING.md.
#![allow(dead_code)]

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::OnceLock;

static DIR: OnceLock<PathBuf> = OnceLock::new();
static SEQ: AtomicU32 = AtomicU32::new(0);

/// The temp data dir of this test binary (created, env set).
pub fn data_dir() -> PathBuf {
    DIR.get_or_init(|| {
        // short: a Unix socket path is limited to ~104 bytes (SUN_LEN) and the tests append their own suffix
        let d = std::env::temp_dir().join(format!("ax-{}", std::process::id()));
        std::fs::create_dir_all(&d).expect("create test data dir");
        std::env::set_var("AXELSTRA_DATA_DIR", &d);
        std::env::set_var("AXELSTRA_TESTING", "1");
        d
    })
    .clone()
}

/// Base pipe name for this test binary (derived from the temp data dir). Tests append their own suffix.
pub fn base_pipe() -> String {
    data_dir();
    axelstra_daemon::proto::pipe_name()
}

/// A pipe name no other test in this process uses.
pub fn pipe(tag: &str) -> String {
    format!("{}-{tag}-{}", base_pipe(), SEQ.fetch_add(1, Ordering::Relaxed)).replace("ThreadId(", "").replace(')', "")
}

/// A fresh projects store (in its own temp dir, so sessions.json and friends stay apart) for `serve_with_store`.
pub fn store(tag: &str) -> PathBuf {
    let d = data_dir().join(format!("{tag}-{}", SEQ.fetch_add(1, Ordering::Relaxed)));
    std::fs::create_dir_all(&d).expect("create test store dir");
    d.join("projects.json")
}
