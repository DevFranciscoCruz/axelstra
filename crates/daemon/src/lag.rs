//! Slow-operation log: anything slower than the threshold is appended to `<data dir>/lag.log` (kept small), so a
//! report of "it stutters" can be matched with what was slow at that moment.
//!
//! The log is written only by [`note`], called when a call or a step has just finished slow: an event, never a timer or
//! a periodic sample, so a daemon nobody talks to writes nothing. [`count`] keeps per-method call totals in memory (a
//! mutex and a map insert per call, no I/O) for `debug.load` (load tests and `tools/idle-load.mjs`).

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::Duration;

/// `(calls, total ms)` per method since the daemon started.
static CALLS: Mutex<BTreeMap<String, (u64, u64)>> = Mutex::new(BTreeMap::new());

/// Counts one finished call of `method`.
pub fn count(method: &str, took: Duration) {
    let mut m = CALLS.lock().unwrap_or_else(|e| e.into_inner());
    let e = match m.get_mut(method) {
        Some(e) => e,
        None => m.entry(method.to_string()).or_default(),
    };
    e.0 += 1;
    e.1 += took.as_millis() as u64;
}

/// Per-method totals: `{method: {calls, ms}}`.
pub fn snapshot() -> serde_json::Value {
    let m = CALLS.lock().unwrap_or_else(|e| e.into_inner());
    m.iter().map(|(k, (n, ms))| (k.clone(), serde_json::json!({"calls": n, "ms": ms}))).collect::<serde_json::Map<_, _>>().into()
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_per_method_without_touching_the_log() {
        count("lagtest.a", Duration::from_millis(5));
        count("lagtest.a", Duration::from_millis(7));
        let s = snapshot();
        assert_eq!(s["lagtest.a"]["calls"], 2);
        assert_eq!(s["lagtest.a"]["ms"], 12);
    }
}
