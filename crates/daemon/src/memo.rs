//! Answers of slow reads that stay valid until something says they are not, shared by the callers that ask at the
//! same moment, and a small pool that bounds how many git/gh/process scans run at once.
//!
//! - **Generation, not a clock.** An entry remembers the sum of the generations of the scopes it depends on
//!   (`*` = anything the app, a flow or an event changed, `wt` = a worktree was added/removed (core's counter), a name
//!   such as `usage`, `gh`, `fs` or a worktree path). Events bump a scope (`Memo::bump`, `bump_all`); an entry is
//!   served only while the sum is unchanged. Where no event exists (files edited in a worktree nobody watches, a port
//!   opened by a dev server) the caller adds `max_age`, checked when the entry is read: a floor, not a timer.
//! - **Single flight.** Callers that ask for the same key at the same generation join the one running the work and
//!   get its result (a failure is not shared: each then runs its own, so errors keep their codes). A caller that
//!   arrives after a bump does not join a run that started before it.
//! - **Pressure.** With `stale_ok` (the memory governor is above Normal) any entry is served, old or not, and nothing
//!   is recomputed; the first call after the level is back recomputes, since the generation moved on meanwhile. Only
//!   an entry from before the last `bump_hard` (a call that changed projects or worktrees) is never served: the caller
//!   of that call must see its own change.
//! - **Pool.** `Pool::run` holds one of N permits while it runs, waiting on a condvar (no polling). A thread that
//!   already holds a permit of the pool runs nested work without another, so it cannot deadlock on itself.

use crate::store::LockRecover;
use anyhow::Result;
use serde_json::Value;
use std::cell::Cell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// What a lookup depends on and how it may be served.
#[derive(Default, Clone, Copy)]
pub struct Opts<'a> {
    /// scopes whose generation the entry follows (`*`, `wt`, or a name)
    pub scopes: &'a [&'a str],
    /// where no event can invalidate: also drop the entry after this long (checked on read)
    pub max_age: Option<Duration>,
    /// serve any entry, even an outdated one, instead of computing (memory pressure)
    pub stale_ok: bool,
}

struct Entry {
    snap: u64,
    /// the hard epoch when the run started
    hard: u64,
    at: Instant,
    value: Value,
}

/// A run in flight. `done` is `None` while it runs, `Some(None)` when it failed, `Some(Some(v))` when it answered.
struct Flight {
    done: Mutex<Option<Option<Value>>>,
    cv: Condvar,
}

impl Flight {
    fn finish(&self, v: Option<Value>) {
        let mut d = self.done.lock_recover();
        if d.is_none() {
            *d = Some(v);
        }
        self.cv.notify_all();
    }

    fn wait(&self) -> Option<Value> {
        let mut d = self.done.lock_recover();
        while d.is_none() {
            d = self.cv.wait(d).unwrap_or_else(|e| e.into_inner());
        }
        d.clone().flatten()
    }
}

const MAX_ENTRIES: usize = 512;

pub struct Memo {
    global: AtomicU64,
    /// moved by calls that change what the user just did (projects, worktrees); stale answers never cross it
    hard: AtomicU64,
    scopes: Mutex<HashMap<String, u64>>,
    entries: Mutex<HashMap<String, Entry>>,
    flights: Mutex<HashMap<String, Arc<Flight>>>,
    /// how many times a computation ran (tests)
    runs: AtomicU64,
}

/// Ends a run: removes it from the table and, if the work panicked before answering, wakes the waiters as failed.
struct Lead<'a> {
    memo: &'a Memo,
    key: String,
    flight: Arc<Flight>,
}

impl Drop for Lead<'_> {
    fn drop(&mut self) {
        self.flight.finish(None);
        self.memo.flights.lock_recover().remove(&self.key);
    }
}

impl Default for Memo {
    fn default() -> Self {
        Self::new()
    }
}

impl Memo {
    pub fn new() -> Memo {
        Memo {
            global: AtomicU64::new(0),
            hard: AtomicU64::new(0),
            scopes: Mutex::default(),
            entries: Mutex::default(),
            flights: Mutex::default(),
            runs: AtomicU64::new(0),
        }
    }

    /// Everything the app, a flow or an event may have changed: drops every entry that depends on `*`.
    pub fn bump_all(&self) {
        self.global.fetch_add(1, Ordering::SeqCst);
    }

    /// A call changed projects or worktrees: also drops the entries a `stale_ok` lookup would still serve.
    pub fn bump_hard(&self) {
        self.hard.fetch_add(1, Ordering::SeqCst);
        self.bump_all();
    }

    /// One scope changed (`usage`, `gh`, `fs`, a worktree path).
    pub fn bump(&self, scope: &str) {
        *self.scopes.lock_recover().entry(scope.to_string()).or_default() += 1;
    }

    fn snapshot(&self, scopes: &[&str]) -> u64 {
        let map = self.scopes.lock_recover();
        scopes.iter().fold(0u64, |acc, s| {
            let g = match *s {
                "*" => self.global.load(Ordering::SeqCst),
                "wt" => axelstra_core::git::worktree_generation(),
                other => map.get(other).copied().unwrap_or(0),
            };
            acc.wrapping_add(g)
        })
    }

    fn lookup(&self, key: &str, snap: u64, hard: u64, o: &Opts) -> Option<Value> {
        let map = self.entries.lock_recover();
        let e = map.get(key)?;
        let fresh = e.snap == snap && o.max_age.is_none_or(|m| e.at.elapsed() < m);
        (fresh || (o.stale_ok && e.hard == hard)).then(|| e.value.clone())
    }

    /// Keeps `value` for `snap`, unless a newer generation's answer is already there (a run that started before a
    /// bump finishing after the run that started after it).
    fn store(&self, key: &str, snap: u64, hard: u64, value: &Value) {
        let mut map = self.entries.lock_recover();
        if map.len() >= MAX_ENTRIES && !map.contains_key(key) {
            map.clear();
        }
        // generations only grow, so a smaller sum is an older answer
        if map.get(key).is_some_and(|e| e.snap > snap) {
            return;
        }
        map.insert(key.to_string(), Entry { snap, hard, at: Instant::now(), value: value.clone() });
    }

    /// The answer for `key`: the kept one while valid, else the result of the run another caller already started for
    /// this generation, else `compute`. Only successes are kept.
    pub fn cached(&self, key: &str, o: Opts, compute: impl FnOnce() -> Result<Value>) -> Result<Value> {
        let snap = self.snapshot(o.scopes);
        let hard = self.hard.load(Ordering::SeqCst);
        if let Some(v) = self.lookup(key, snap, hard, &o) {
            return Ok(v);
        }
        let fkey = format!("{key}@{snap}");
        let (flight, leader) = {
            let mut f = self.flights.lock_recover();
            match f.get(&fkey) {
                Some(fl) => (fl.clone(), false),
                None => {
                    let fl = Arc::new(Flight { done: Mutex::new(None), cv: Condvar::new() });
                    f.insert(fkey.clone(), fl.clone());
                    (fl, true)
                }
            }
        };
        if !leader {
            if let Some(v) = flight.wait() {
                return Ok(v);
            }
            return self.run(key, snap, hard, compute);
        }
        let _lead = Lead { memo: self, key: fkey, flight: flight.clone() };
        let res = self.run(key, snap, hard, compute);
        flight.finish(res.as_ref().ok().cloned());
        res
    }

    fn run(&self, key: &str, snap: u64, hard: u64, compute: impl FnOnce() -> Result<Value>) -> Result<Value> {
        self.runs.fetch_add(1, Ordering::SeqCst);
        let v = compute()?;
        // kept under the generation read BEFORE the work: a bump meanwhile makes it stale at once, which is right
        self.store(key, snap, hard, &v);
        Ok(v)
    }

    #[cfg(test)]
    fn runs(&self) -> u64 {
        self.runs.load(Ordering::SeqCst)
    }
}

/// The process-wide cache of the daemon (keys carry the registry's address, so daemons hosted by one test process do
/// not share answers).
pub fn memo() -> &'static Memo {
    static M: OnceLock<Memo> = OnceLock::new();
    M.get_or_init(Memo::new)
}

/// Bounded concurrency for work that starts git, gh or a process scan.
pub struct Pool {
    free: Mutex<usize>,
    cv: Condvar,
}

thread_local! {
    /// address of the pool this thread holds a permit of (0 = none)
    static HELD: Cell<usize> = const { Cell::new(0) };
}

struct Permit<'a>(&'a Pool);

impl Drop for Permit<'_> {
    fn drop(&mut self) {
        HELD.with(|h| h.set(0));
        *self.0.free.lock_recover() += 1;
        self.0.cv.notify_one();
    }
}

impl Pool {
    pub fn new(permits: usize) -> Pool {
        Pool { free: Mutex::new(permits.max(1)), cv: Condvar::new() }
    }

    /// Runs `f` holding a permit; waits for one if all are taken.
    pub fn run<R>(&self, f: impl FnOnce() -> R) -> R {
        let me = self as *const Pool as usize;
        if HELD.with(|h| h.get()) == me {
            return f();
        }
        {
            let mut free = self.free.lock_recover();
            while *free == 0 {
                free = self.cv.wait(free).unwrap_or_else(|e| e.into_inner());
            }
            *free -= 1;
        }
        HELD.with(|h| h.set(me));
        let _permit = Permit(self);
        f()
    }
}

/// Floors for answers that depend on something no event reports (see the module comment): files edited in a worktree
/// nobody watches, a worktree added by the git CLI, a port opened by a dev server, GitHub changing on its own.
pub const WORKTREE_MAX_AGE: Duration = Duration::from_secs(30);
pub const PORTS_MAX_AGE: Duration = Duration::from_secs(10);
pub const GH_MAX_AGE: Duration = Duration::from_secs(120);
pub const BUDGET_MAX_AGE: Duration = Duration::from_secs(120);

/// Permits of the daemon's git pool: scans mostly wait on child processes and the disk, and a few at a time are
/// enough to keep the screen current without every core going to kernel time.
const GIT_PERMITS: usize = 3;

pub fn git_pool() -> &'static Pool {
    static P: OnceLock<Pool> = OnceLock::new();
    P.get_or_init(|| Pool::new(GIT_PERMITS))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::AtomicUsize;

    const ALL: Opts<'static> = Opts { scopes: &["*"], max_age: None, stale_ok: false };

    /// A computation that signals when it started and then waits to be released.
    struct Gate {
        started: (Mutex<bool>, Condvar),
        release: (Mutex<bool>, Condvar),
    }

    impl Gate {
        fn new() -> Arc<Gate> {
            Arc::new(Gate { started: (Mutex::new(false), Condvar::new()), release: (Mutex::new(false), Condvar::new()) })
        }

        fn hold(&self) {
            *self.started.0.lock().unwrap() = true;
            self.started.1.notify_all();
            let mut r = self.release.0.lock().unwrap();
            while !*r {
                r = self.release.1.wait(r).unwrap();
            }
        }

        fn wait_started(&self) {
            let mut s = self.started.0.lock().unwrap();
            while !*s {
                s = self.started.1.wait(s).unwrap();
            }
        }

        fn open(&self) {
            *self.release.0.lock().unwrap() = true;
            self.release.1.notify_all();
        }
    }

    #[test]
    fn a_kept_answer_is_served_until_its_scope_moves() {
        let m = Memo::new();
        assert_eq!(m.cached("k", ALL, || Ok(json!(1))).unwrap(), json!(1));
        assert_eq!(m.cached("k", ALL, || Ok(json!(2))).unwrap(), json!(1));
        m.bump("other");
        assert_eq!(m.cached("k", ALL, || Ok(json!(3))).unwrap(), json!(1));
        m.bump_all();
        assert_eq!(m.cached("k", ALL, || Ok(json!(4))).unwrap(), json!(4));
        assert_eq!(m.runs(), 2);
    }

    #[test]
    fn a_named_scope_invalidates_only_its_dependents() {
        let m = Memo::new();
        let usage = Opts { scopes: &["usage"], ..Default::default() };
        m.cached("u", usage, || Ok(json!("a"))).unwrap();
        m.bump_all();
        assert_eq!(m.cached("u", usage, || Ok(json!("b"))).unwrap(), json!("a"));
        m.bump("usage");
        assert_eq!(m.cached("u", usage, || Ok(json!("c"))).unwrap(), json!("c"));
    }

    #[test]
    fn max_age_is_a_floor_where_no_event_exists() {
        let m = Memo::new();
        let o = Opts { scopes: &["*"], max_age: Some(Duration::from_millis(30)), stale_ok: false };
        m.cached("k", o, || Ok(json!(1))).unwrap();
        assert_eq!(m.cached("k", o, || Ok(json!(2))).unwrap(), json!(1));
        std::thread::sleep(Duration::from_millis(80));
        assert_eq!(m.cached("k", o, || Ok(json!(3))).unwrap(), json!(3));
    }

    #[test]
    fn failures_are_not_kept() {
        let m = Memo::new();
        assert!(m.cached("k", ALL, || Err(anyhow::anyhow!("boom"))).is_err());
        assert_eq!(m.cached("k", ALL, || Ok(json!(7))).unwrap(), json!(7));
    }

    #[test]
    fn identical_calls_in_flight_join_the_leader() {
        let m = Arc::new(Memo::new());
        let gate = Gate::new();
        let calls = Arc::new(AtomicUsize::new(0));
        let lead = {
            let (m, gate, calls) = (m.clone(), gate.clone(), calls.clone());
            std::thread::spawn(move || {
                m.cached("k", ALL, || {
                    calls.fetch_add(1, Ordering::SeqCst);
                    gate.hold();
                    Ok(json!("lead"))
                })
                .unwrap()
            })
        };
        gate.wait_started();
        let followers: Vec<_> = (0..4)
            .map(|_| {
                let (m, calls) = (m.clone(), calls.clone());
                std::thread::spawn(move || {
                    m.cached("k", ALL, || {
                        calls.fetch_add(1, Ordering::SeqCst);
                        Ok(json!("follower"))
                    })
                    .unwrap()
                })
            })
            .collect();
        // the leader is still held: a follower that ran its own work would have counted by now
        std::thread::sleep(Duration::from_millis(150));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        gate.open();
        assert_eq!(lead.join().unwrap(), json!("lead"));
        for f in followers {
            assert_eq!(f.join().unwrap(), json!("lead"));
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_call_after_a_bump_does_not_join_an_older_run() {
        let m = Arc::new(Memo::new());
        let gate = Gate::new();
        let lead = {
            let (m, gate) = (m.clone(), gate.clone());
            std::thread::spawn(move || {
                m.cached("k", ALL, || {
                    gate.hold();
                    Ok(json!("old"))
                })
                .unwrap()
            })
        };
        gate.wait_started();
        m.bump_all();
        // the mutation already happened: this answer must be computed after it
        assert_eq!(m.cached("k", ALL, || Ok(json!("new"))).unwrap(), json!("new"));
        gate.open();
        assert_eq!(lead.join().unwrap(), json!("old"));
        // the old run's answer does not replace the newer one
        assert_eq!(m.cached("k", ALL, || panic!("kept")).unwrap(), json!("new"));
    }

    #[test]
    fn under_pressure_the_kept_answer_is_served_even_when_outdated_and_nothing_runs() {
        let m = Memo::new();
        m.cached("k", ALL, || Ok(json!("kept"))).unwrap();
        m.bump_all();
        let o = Opts { stale_ok: true, ..ALL };
        let got = m.cached("k", o, || panic!("must not compute at High")).unwrap();
        assert_eq!(got, json!("kept"));
        // nothing kept yet: it has to answer
        assert_eq!(m.cached("fresh", o, || Ok(json!("computed"))).unwrap(), json!("computed"));
        // back to normal: recomputed
        assert_eq!(m.cached("k", ALL, || Ok(json!("now"))).unwrap(), json!("now"));
    }

    #[test]
    fn under_pressure_an_answer_from_before_a_hard_change_is_not_served() {
        let m = Memo::new();
        let o = Opts { stale_ok: true, ..ALL };
        m.cached("k", o, || Ok(json!("before"))).unwrap();
        m.bump_hard();
        // the caller changed projects: it must see its own change even at High
        assert_eq!(m.cached("k", o, || Ok(json!("after"))).unwrap(), json!("after"));
        // and from then on the new answer is the stale one served
        m.bump_all();
        assert_eq!(m.cached("k", o, || panic!("must not compute at High")).unwrap(), json!("after"));
    }

    #[test]
    fn the_pool_never_runs_more_than_its_permits() {
        let pool = Arc::new(Pool::new(2));
        let (now, peak) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
        let hs: Vec<_> = (0..8)
            .map(|_| {
                let (pool, now, peak) = (pool.clone(), now.clone(), peak.clone());
                std::thread::spawn(move || {
                    pool.run(|| {
                        let n = now.fetch_add(1, Ordering::SeqCst) + 1;
                        peak.fetch_max(n, Ordering::SeqCst);
                        std::thread::sleep(Duration::from_millis(20));
                        now.fetch_sub(1, Ordering::SeqCst);
                    })
                })
            })
            .collect();
        for h in hs {
            h.join().unwrap();
        }
        assert_eq!(peak.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn nested_work_on_the_same_pool_does_not_deadlock() {
        let pool = Pool::new(1);
        assert_eq!(pool.run(|| pool.run(|| 5)), 5);
        // the permit came back
        assert_eq!(pool.run(|| 6), 6);
    }
}
