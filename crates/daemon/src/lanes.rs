//! Slow reads (git, process scans, directory walks) run off the connection's ordered call worker, in a bounded pool,
//! and identical calls in flight share one run. Everything else stays in order on the worker, so a `ping`, a
//! `settings.get` or a `session.list` never queues behind a `git.changes` that takes seconds.

use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use tokio::sync::{oneshot, Semaphore};

/// Which pool a slow read uses; `project.list` has its own so a burst of git reads cannot starve it.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Lane {
    Git,
    Projects,
    /// `project.logo`: disk reads of a repo's logo files, which can hang on a dead mount; two at a time, never the git pool
    Logo,
}

/// The lane of a method that only reads and may be slow, or `None` for a method that runs in order on the worker.
pub fn lane_of(method: &str) -> Option<Lane> {
    match method {
        "project.list" => Some(Lane::Projects),
        "project.logo" => Some(Lane::Logo),
        "git.changes"
        | "git.log"
        | "git.show"
        | "git.diff"
        | "git.timeline"
        | "worktree.status"
        | "worktree.overview"
        | "dispatch.overlaps"
        | "dispatch.overlap_files"
        | "ports.list"
        | "fs.list"
        | "fs.search"
        | "fs.grep" => Some(Lane::Git),
        "debug.slow" if std::env::var_os("AXELSTRA_TESTING").is_some() => Some(Lane::Git),
        _ => None,
    }
}

/// After this long a slow read answers `{"loading": true}` (the work goes on and a repeat joins it), so a caller with a
/// 30 s limit gets a result instead of a hang. `project.list` answers its last known list instead, when it has one.
pub fn answer_limit(lane: Lane) -> Duration {
    match lane {
        Lane::Git => Duration::from_secs(20),
        Lane::Projects => Duration::from_secs(3),
        Lane::Logo => Duration::from_secs(5),
    }
}

const GIT_PERMITS: usize = 6;
const PROJECT_PERMITS: usize = 2;
const LOGO_PERMITS: usize = 2;

pub fn gate(lane: Lane) -> &'static Semaphore {
    static GIT: OnceLock<Semaphore> = OnceLock::new();
    static PROJECTS: OnceLock<Semaphore> = OnceLock::new();
    static LOGO: OnceLock<Semaphore> = OnceLock::new();
    match lane {
        Lane::Git => GIT.get_or_init(|| Semaphore::new(GIT_PERMITS)),
        Lane::Projects => PROJECTS.get_or_init(|| Semaphore::new(PROJECT_PERMITS)),
        Lane::Logo => LOGO.get_or_init(|| Semaphore::new(LOGO_PERMITS)),
    }
}

type Waiters = Vec<oneshot::Sender<Value>>;

fn flights() -> &'static Mutex<HashMap<String, Waiters>> {
    static F: OnceLock<Mutex<HashMap<String, Waiters>>> = OnceLock::new();
    F.get_or_init(Default::default)
}

/// Runs `work` unless an identical call (same `key`) is already in flight, in which case this waits for that one's
/// result. The leader's failure is not shared: each waiter then runs its own call, so errors keep their codes.
pub async fn single_flight<F, Fut>(key: String, work: F) -> anyhow::Result<Value>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = anyhow::Result<Value>>,
{
    let waiter = {
        let mut f = flights().lock().unwrap_or_else(|e| e.into_inner());
        match f.get_mut(&key) {
            Some(w) => {
                let (s, r) = oneshot::channel();
                w.push(s);
                Some(r)
            }
            None => {
                f.insert(key.clone(), Vec::new());
                None
            }
        }
    };
    if let Some(r) = waiter {
        if let Ok(v) = r.await {
            return Ok(v);
        }
        return work().await;
    }
    let res = work().await;
    let waiters = flights().lock().unwrap_or_else(|e| e.into_inner()).remove(&key).unwrap_or_default();
    if let Ok(v) = &res {
        for w in waiters {
            let _ = w.send(v.clone());
        }
    }
    res
}
