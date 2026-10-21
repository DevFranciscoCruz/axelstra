//! Slow-operation log: anything slower than the threshold is appended to `<data dir>/lag.log` (kept small), so a
//! report of "it stutters" can be matched with what was slow at that moment.

use std::time::Duration;

pub fn note(what: &str, took: Duration, min_ms: u128) {
    if took.as_millis() < min_ms {
        return;
    }
    let path = crate::proto::data_dir().join("lag.log");
    // start over when it grows past 256 KB
    if std::fs::metadata(&path).map(|m| m.len() > 256 * 1024).unwrap_or(false) {
        let _ = std::fs::remove_file(&path);
    }
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        use std::io::Write;
        let _ = writeln!(f, "{now} {what}: {} ms", took.as_millis());
    }
}
