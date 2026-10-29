//! Is the running daemon from another build than this app, and may the app replace it? A daemon outlives app updates
//! and most changes do not bump `PROTO`, so the app compares commits. Pure: the facts (the two builds, where the daemon
//! runs from, what its sessions are doing) are gathered by `connect` in `main.rs`.

use serde_json::Value;
use std::path::Path;

/// What a build says about itself (`axelstra_core::buildinfo`, or the daemon's `hello`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Build {
    pub version: String,
    pub commit: String,
}

impl Build {
    /// The commit, unless it is missing or "unknown" (a daemon from before `hello` said it, a build without git).
    pub fn known_commit(&self) -> Option<&str> {
        let c = self.commit.trim();
        (!c.is_empty() && c != "unknown").then_some(c)
    }

    /// "1.2.3 (abc1234)", "unknown" for what is missing; for the log line.
    pub fn label(&self) -> String {
        let or = |s: &str| if s.trim().is_empty() { "unknown".to_string() } else { s.trim().to_string() };
        format!("{} ({})", or(&self.version), or(&self.commit))
    }
}

/// Where the daemon's executable is, seen from the app's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    /// in the app's own directory: the daemon `start_daemon` would launch
    Beside,
    /// elsewhere (a `cargo run` daemon, one the user started by hand)
    Elsewhere,
    /// the OS did not say and the daemon did not report it
    Unknown,
}

/// What the daemon's sessions are doing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Busy {
    Idle,
    Working,
    /// `session.list` failed: counted as busy
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    /// something works, waits for an answer, awaits, is paused or hibernated
    Busy,
    /// the daemon is not the one beside the app (or where it runs is not known)
    Dev,
}

impl Reason {
    pub fn as_str(self) -> &'static str {
        match self {
            Reason::Busy => "busy",
            Reason::Dev => "dev",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    /// same build, or a commit is not known: keep it (the behaviour from before this check)
    Keep,
    /// stale, beside the app and idle: shut it down and start the app's own
    Replace,
    /// stale, but replacing it now could lose work or would start another build: keep it, offer a restart
    Offer(Reason),
}

/// The rule. Only a daemon whose commit is known and differs from the app's is stale; a stale one is replaced only when
/// it runs from the app's directory and nothing in it is busy. The too-old rule (`PROTO`, `SECURITY_MIN`) is applied
/// before this and always replaces.
pub fn decide(app: &Build, daemon: &Build, place: Place, busy: Busy) -> Decision {
    let (Some(a), Some(d)) = (app.known_commit(), daemon.known_commit()) else { return Decision::Keep };
    if a.eq_ignore_ascii_case(d) {
        return Decision::Keep;
    }
    if place != Place::Beside {
        return Decision::Offer(Reason::Dev);
    }
    match busy {
        Busy::Idle => Decision::Replace,
        Busy::Working | Busy::Unknown => Decision::Offer(Reason::Busy),
    }
}

/// Where `daemon_exe` is relative to `app_exe`: the canonical parent directories compared.
pub fn place(app_exe: Option<&Path>, daemon_exe: Option<&Path>) -> Place {
    let dir = |p: &Path| p.parent().map(|d| std::fs::canonicalize(d).unwrap_or_else(|_| d.to_path_buf()));
    match (app_exe.and_then(dir), daemon_exe.and_then(dir)) {
        (Some(a), Some(d)) if a == d => Place::Beside,
        (Some(_), Some(_)) => Place::Elsewhere,
        _ => Place::Unknown,
    }
}

/// A session counts as working for this long after its last output (the frontend's `WORKING_MS`).
const WORKING_MS: u64 = 3000;

/// Busy unless every running row of `session.list` is quiet: not working (output in the last `WORKING_MS`), not
/// waiting for an answer, not awaiting, not paused, not hibernated. Agents and terminals alike.
pub fn busy_of(rows: &Value) -> Busy {
    let Some(rows) = rows.as_array() else { return Busy::Unknown };
    let busy = rows.iter().filter(|r| r["running"] != false).any(|r| {
        let set = |k: &str| !r[k].is_null();
        r["idle_ms"].as_u64().is_none_or(|ms| ms < WORKING_MS)
            || r["waiting"] == true
            || set("wait")
            || set("awaiting")
            || set("paused")
            || set("hibernated")
    });
    if busy {
        Busy::Working
    } else {
        Busy::Idle
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn b(commit: &str) -> Build {
        Build { version: "1.0.0".into(), commit: commit.into() }
    }

    #[test]
    fn same_commit_keeps_whatever_else() {
        for place in [Place::Beside, Place::Elsewhere, Place::Unknown] {
            for busy in [Busy::Idle, Busy::Working, Busy::Unknown] {
                assert_eq!(decide(&b("abc1234"), &b("abc1234"), place, busy), Decision::Keep);
                assert_eq!(decide(&b("ABC1234"), &b("abc1234"), place, busy), Decision::Keep);
            }
        }
    }

    #[test]
    fn an_unknown_commit_on_either_side_keeps() {
        for (app, daemon) in [("abc1234", ""), ("abc1234", "unknown"), ("", "abc1234"), ("unknown", "abc1234"), ("", ""), (" ", "x")] {
            for place in [Place::Beside, Place::Elsewhere, Place::Unknown] {
                for busy in [Busy::Idle, Busy::Working, Busy::Unknown] {
                    assert_eq!(decide(&b(app), &b(daemon), place, busy), Decision::Keep, "{app:?} {daemon:?} {place:?} {busy:?}");
                }
            }
        }
    }

    #[test]
    fn stale_beside_and_idle_replaces() {
        assert_eq!(decide(&b("abc1234"), &b("def5678"), Place::Beside, Busy::Idle), Decision::Replace);
    }

    #[test]
    fn stale_beside_but_busy_offers() {
        for busy in [Busy::Working, Busy::Unknown] {
            assert_eq!(decide(&b("abc1234"), &b("def5678"), Place::Beside, busy), Decision::Offer(Reason::Busy));
        }
    }

    #[test]
    fn stale_elsewhere_or_unknown_place_offers_dev_never_replaces() {
        for place in [Place::Elsewhere, Place::Unknown] {
            for busy in [Busy::Idle, Busy::Working, Busy::Unknown] {
                assert_eq!(decide(&b("abc1234"), &b("def5678"), place, busy), Decision::Offer(Reason::Dev));
            }
        }
    }

    #[test]
    fn place_compares_canonical_directories() {
        let dir = std::env::temp_dir().join(format!("axelstra-stale-{}", std::process::id()));
        let other = dir.join("other");
        std::fs::create_dir_all(&other).unwrap();
        let app = dir.join("axelstra-app.exe");
        assert_eq!(place(Some(&app), Some(&dir.join("axelstra.exe"))), Place::Beside);
        // the same directory spelled through a `..`
        assert_eq!(place(Some(&app), Some(&other.join("..").join("axelstra.exe"))), Place::Beside);
        assert_eq!(place(Some(&app), Some(&other.join("axelstra.exe"))), Place::Elsewhere);
        assert_eq!(place(Some(&app), None), Place::Unknown);
        assert_eq!(place(None, Some(&app)), Place::Unknown);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn row(extra: Value) -> Value {
        let mut r = json!({"id": 1, "running": true, "idle_ms": 60_000, "waiting": false, "wait": null, "awaiting": null,
            "paused": null, "hibernated": null});
        for (k, v) in extra.as_object().unwrap() {
            r[k] = v.clone();
        }
        r
    }

    #[test]
    fn busy_reads_each_state_of_a_row() {
        assert_eq!(busy_of(&json!([])), Busy::Idle);
        assert_eq!(busy_of(&json!([row(json!({}))])), Busy::Idle);
        // an ended session does not count, whatever it last did
        assert_eq!(busy_of(&json!([row(json!({"running": false, "idle_ms": 0}))])), Busy::Idle);
        for extra in [
            json!({"idle_ms": 100}),
            json!({"idle_ms": null}),
            json!({"waiting": true}),
            json!({"wait": {"kind": "permission", "text": "?"}}),
            json!({"awaiting": {"reason": "CI", "since": 1, "until": null}}),
            json!({"paused": {"reason": "memory", "since": 1, "build": false}}),
            json!({"hibernated": {"since": 1, "saved_mb": 10, "waking": false, "queued": false}}),
        ] {
            assert_eq!(busy_of(&json!([row(json!({})), row(extra.clone())])), Busy::Working, "{extra}");
        }
        assert_eq!(busy_of(&json!({"error": "x"})), Busy::Unknown);
    }

    #[test]
    fn label_names_what_is_missing() {
        assert_eq!(b("abc1234").label(), "1.0.0 (abc1234)");
        assert_eq!(Build::default().label(), "unknown (unknown)");
    }
}
