//! Audit log of what agents ask the daemon to do: a bounded ring in memory (what `audit.list` answers) mirrored to a
//! size-capped JSON-lines file in the data dir, so the user can see afterwards what an agent did and when.

use crate::store::LockRecover;
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

/// Entries kept in memory.
const RING: usize = 500;
/// The file is rotated to `<name>.1` once it passes this size, so it never grows without bound.
const FILE_MAX: u64 = 1 << 20;
/// Longest text kept from one parameter.
const FIELD_MAX: usize = 200;

/// Calls that only look at state: not worth a line each (an agent polls them).
fn is_noise(method: &str) -> bool {
    let tail = method.rsplit('.').next().unwrap_or(method);
    matches!(method, "ping" | "hello" | "audit.list" | "notices.take")
        || matches!(tail, "list" | "status" | "get" | "read" | "inbox" | "screen" | "resources" | "overview" | "thread")
}

fn cut(s: &str) -> String {
    if s.chars().count() > FIELD_MAX {
        format!("{}…", s.chars().take(FIELD_MAX).collect::<String>())
    } else {
        s.to_string()
    }
}

/// The params as a short, secret-free summary: strings cut to `FIELD_MAX`, credentials dropped, nesting flattened.
pub fn summarize(params: &Value) -> Value {
    let Some(obj) = params.as_object() else { return Value::Null };
    let mut out = serde_json::Map::new();
    for (k, v) in obj {
        if k.contains("token") || k.contains("secret") || k == "env" {
            continue;
        }
        let v = match v {
            Value::String(s) => json!(cut(s)),
            Value::Number(_) | Value::Bool(_) | Value::Null => v.clone(),
            other => json!(cut(&other.to_string())),
        };
        out.insert(k.clone(), v);
    }
    Value::Object(out)
}

pub struct Audit {
    ring: Mutex<VecDeque<Value>>,
    file: Option<PathBuf>,
}

impl Audit {
    /// `file`: where entries are appended (None keeps them in memory only).
    pub fn new(file: Option<PathBuf>) -> Audit {
        let ring = file.as_deref().map(load_tail).unwrap_or_default();
        Audit { ring: Mutex::new(ring), file }
    }

    /// Records one call made over an agent connection (`session` is the session its credential belongs to).
    pub fn record(&self, session: u32, method: &str, params: &Value, outcome: &Result<(), String>) {
        if is_noise(method) {
            return;
        }
        let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let mut e = json!({"ts": ts, "session": session, "method": method, "params": summarize(params), "ok": outcome.is_ok()});
        if let Err(msg) = outcome {
            e["error"] = json!(cut(msg));
        }
        {
            let mut ring = self.ring.lock_recover();
            if ring.len() >= RING {
                ring.pop_front();
            }
            ring.push_back(e.clone());
        }
        if let Some(f) = &self.file {
            append(f, &e);
        }
    }

    /// Records something the daemon did on its own, not on a client's request (session 0): trigger firings.
    pub fn record_system(&self, method: &str, params: &Value, outcome: &Result<(), String>) {
        self.record(0, method, params, outcome)
    }

    /// The newest `limit` entries (oldest first), optionally only those of one session.
    pub fn list(&self, limit: usize, session: Option<u32>) -> Vec<Value> {
        let ring = self.ring.lock_recover();
        let mut v: Vec<Value> = ring
            .iter()
            .rev()
            .filter(|e| session.is_none_or(|s| e["session"].as_u64() == Some(u64::from(s))))
            .take(limit)
            .cloned()
            .collect();
        v.reverse();
        v
    }
}

fn append(path: &std::path::Path, e: &Value) {
    if std::fs::metadata(path).is_ok_and(|m| m.len() > FILE_MAX) {
        let mut old = path.as_os_str().to_owned();
        old.push(".1");
        let _ = std::fs::remove_file(&old);
        let _ = std::fs::rename(path, &old);
    }
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{e}");
    }
}

/// The last `RING` entries of an existing log, so a restarted daemon still shows what happened before.
fn load_tail(path: &std::path::Path) -> VecDeque<Value> {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let all: Vec<Value> = text.lines().filter_map(|l| serde_json::from_str(l).ok()).collect();
    all.into_iter().rev().take(RING).rev().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_calls_hides_secrets_and_skips_polling() {
        let a = Audit::new(None);
        a.record(3, "session.list", &json!({}), &Ok(()));
        a.record(3, "session.spawn", &json!({"program": "sh", "agent_token": "s3cret", "env": {"A": "b"}}), &Ok(()));
        a.record(4, "fs.delete", &json!({"path": "x"}), &Err("no".into()));
        let all = a.list(10, None);
        assert_eq!(all.len(), 2);
        assert_eq!(all[0]["method"], "session.spawn");
        assert!(all[0]["params"].get("agent_token").is_none() && all[0]["params"].get("env").is_none());
        assert_eq!(all[1]["ok"], false);
        assert_eq!(a.list(10, Some(4)).len(), 1);
        assert_eq!(a.list(1, None)[0]["method"], "fs.delete");
    }

    #[test]
    fn ring_is_bounded_and_file_is_replayed() {
        let dir = std::env::temp_dir().join(format!("axl-audit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let f = dir.join("audit.log");
        let a = Audit::new(Some(f.clone()));
        for i in 0..(RING + 20) {
            a.record(1, "msg.send", &json!({"text": format!("m{i}")}), &Ok(()));
        }
        assert_eq!(a.list(10_000, None).len(), RING);
        let b = Audit::new(Some(f));
        let back = b.list(10_000, None);
        assert_eq!(back.len(), RING);
        assert_eq!(back.last().unwrap()["params"]["text"], format!("m{}", RING + 19));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn long_text_is_cut() {
        let s = summarize(&json!({"text": "x".repeat(1000)}));
        assert!(s["text"].as_str().unwrap().chars().count() <= FIELD_MAX + 1);
    }
}
