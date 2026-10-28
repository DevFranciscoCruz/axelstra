//! What the daemon knows of the pull requests of its worktrees, kept fresh by events. A PR changes on GitHub, where nothing
//! can push to us, so three kinds of wake-up stand in for a timer:
//!
//! - local events that usually come with a change: a ref moved (`RefWatch`: a fetch, a push, a commit), an agent said
//!   `done`, a merge, a session ended, the window got focus (`gh.pr_refresh`);
//! - the app asking about a worktree (`git.checks` records what it saw and keeps the worktree "of interest");
//! - ONE computed deadline, the earliest moment a known PR is due a new look (`Table::next_deadline`): a minute while its
//!   CI runs, a quarter of an hour while it is open and settled, never for a merged, closed or unreadable one. With
//!   nothing due there is no timer at all.
//!
//! `Table` is the pure part (what was seen, what is stale, when to look again); the daemon loop in `server/triggers.rs`
//! evaluates, and tells the app with `pr.changed {worktree, number, state, ci}` when the answer differs from the last one.

use crate::checks::Status;
use crate::store::LockRecover;
use axelstra_core::paths;
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, LazyLock, Mutex};
use std::time::Duration;

/// Next look while the CI of an open PR runs.
pub const PENDING_MS: u64 = 60_000;
/// Next look at an open PR whose CI is settled (a review or a merge can still come, with no local event).
pub const OPEN_MS: u64 = 15 * 60_000;
/// An event re-reads an open PR only when the last look is older than this (events come in bursts).
pub const MIN_AGE_MS: u64 = 10_000;
/// An event re-reads a merged, closed or unreadable PR only when the last look is older than this.
pub const TERMINAL_AGE_MS: u64 = 5 * 60_000;
/// A worktree stays "of interest" this long after the app or the triggers last looked at it.
pub const INTEREST_MS: u64 = 30 * 60_000;
/// Worktrees kept at most.
const CAP: usize = 32;

/// Wakes the PR loop; the count says whether something happened since the last wait.
pub struct Wake {
    n: Mutex<u64>,
    cv: Condvar,
}

impl Wake {
    pub const fn new() -> Self {
        Wake { n: Mutex::new(0), cv: Condvar::new() }
    }

    pub fn bump(&self) {
        *self.n.lock_recover() += 1;
        self.cv.notify_all();
    }

    pub fn now(&self) -> u64 {
        *self.n.lock_recover()
    }

    /// Waits for a bump after `seen`, or at most `timeout` (None: only a bump ends it). Returns the count now.
    pub fn wait(&self, seen: u64, timeout: Option<Duration>) -> u64 {
        let g = self.n.lock_recover();
        let g = match timeout {
            None => self.cv.wait_while(g, |n| *n == seen).unwrap_or_else(|e| e.into_inner()),
            Some(t) => self.cv.wait_timeout_while(g, t, |n| *n == seen).unwrap_or_else(|e| e.into_inner()).0,
        };
        *g
    }
}

impl Default for Wake {
    fn default() -> Self {
        Self::new()
    }
}

pub static WAKE: Wake = Wake::new();

/// The shared table: the loop evaluates into it, `git.checks` records into it.
pub static TABLE: LazyLock<Mutex<Table>> = LazyLock::new(|| Mutex::new(Table::default()));

pub fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// What makes two answers "different" for the app.
#[derive(Debug, Clone, PartialEq)]
pub struct Key {
    problem: String,
    number: u64,
    state: String,
    draft: bool,
    ci: String,
    sha: String,
    review: String,
    mergeable: String,
    merge_state: String,
    checks: Vec<(String, String)>,
}

pub fn key_of(s: &Status) -> Key {
    Key {
        problem: s.problem.clone(),
        number: s.number,
        state: s.pr_state.clone(),
        draft: s.draft,
        ci: s.ci.clone(),
        sha: s.head_sha.clone(),
        review: s.review.clone(),
        mergeable: s.mergeable.clone(),
        merge_state: s.merge_state.clone(),
        checks: s.checks.iter().map(|c| (c.name.clone(), c.state.clone())).collect(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Observed {
    /// never seen before: nobody can have an older answer from us
    First,
    Same,
    Changed,
}

/// Milliseconds until a PR in this state wants a new look without any event; `None`: it does not.
pub fn interval_ms(s: &Status) -> Option<u64> {
    if !s.problem.is_empty() || s.pr_state != "open" {
        return None;
    }
    Some(if s.ci == "pending" { PENDING_MS } else { OPEN_MS })
}

struct Entry {
    path: PathBuf,
    status: Status,
    key: Key,
    checked: u64,
    until: u64,
}

impl Entry {
    /// When it is due without an event.
    fn due(&self) -> Option<u64> {
        interval_ms(&self.status).map(|i| self.checked.saturating_add(i))
    }

    /// Whether the last look still counts: on an event only a young one does, otherwise until the deadline.
    fn fresh(&self, now: u64, event: bool) -> bool {
        let age = now.saturating_sub(self.checked);
        if event {
            age < if interval_ms(&self.status).is_some() { MIN_AGE_MS } else { TERMINAL_AGE_MS }
        } else {
            self.due().is_none_or(|d| now < d)
        }
    }
}

#[derive(Default)]
pub struct Table {
    map: HashMap<String, Entry>,
}

impl Table {
    /// Records what a look found (the loop's, or the app's own `git.checks`) and keeps the worktree of interest.
    pub fn note(&mut self, path: &Path, st: &Status, now: u64) -> Observed {
        let k = paths::norm(&path.to_string_lossy());
        let key = key_of(st);
        let out = match self.map.get_mut(&k) {
            Some(e) => {
                let changed = e.key != key;
                e.status = st.clone();
                e.key = key;
                e.checked = now;
                e.until = now.saturating_add(INTEREST_MS);
                e.path = path.to_path_buf();
                if changed {
                    Observed::Changed
                } else {
                    Observed::Same
                }
            }
            None => {
                self.map.insert(
                    k,
                    Entry { path: path.to_path_buf(), status: st.clone(), key, checked: now, until: now.saturating_add(INTEREST_MS) },
                );
                Observed::First
            }
        };
        if self.map.len() > CAP {
            if let Some(old) = self.map.iter().min_by_key(|(_, e)| e.checked).map(|(k, _)| k.clone()) {
                self.map.remove(&old);
            }
        }
        out
    }

    /// The last answer for this worktree when it still counts (see `Entry::fresh`).
    pub fn fresh(&self, path: &Path, now: u64, event: bool) -> Option<Status> {
        self.map.get(&paths::norm(&path.to_string_lossy())).filter(|e| now < e.until && e.fresh(now, event)).map(|e| e.status.clone())
    }

    /// Worktrees of interest whose last look no longer counts.
    pub fn stale(&self, now: u64, event: bool) -> Vec<PathBuf> {
        self.map.values().filter(|e| now < e.until && !e.fresh(now, event)).map(|e| e.path.clone()).collect()
    }

    /// Milliseconds to the earliest moment a worktree of interest is due (0: already), `None` when nothing is.
    pub fn next_deadline(&self, now: u64) -> Option<u64> {
        self.map.values().filter(|e| now < e.until).filter_map(Entry::due).map(|d| d.saturating_sub(now)).min()
    }

    /// Forgets the worktrees nobody looked at for the interest spell.
    pub fn prune(&mut self, now: u64) {
        self.map.retain(|_, e| now < e.until);
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

/// A look the app made itself (`git.checks`): remember it, and say whether it differs from what the daemon knew.
pub fn note_seen(path: &Path, st: &Status) -> Observed {
    let o = TABLE.lock_recover().note(path, st, now_ms());
    // the loop works out its deadline again
    WAKE.bump();
    o
}

// ---------- refs ----------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefKind {
    /// a remote-tracking ref or `packed-refs` moved: a fetch or a push brought news
    Remote,
    /// a local branch or `HEAD` moved
    Local,
    /// `FETCH_HEAD` was written: a fetch ran (it may have brought nothing)
    Fetch,
}

/// What a changed path inside a git directory means; `None` for everything else (lock files, objects, logs).
pub fn classify(path: &Path) -> Option<RefKind> {
    let s = path.to_string_lossy().replace('\\', "/");
    let name = s.rsplit('/').next()?;
    if name.ends_with(".lock") {
        return None;
    }
    match name {
        "FETCH_HEAD" => return Some(RefKind::Fetch),
        "packed-refs" => return Some(RefKind::Remote),
        _ => {}
    }
    if let Some(i) = s.find("/refs/") {
        let rest = &s[i + 6..];
        return if rest.starts_with("remotes/") {
            Some(RefKind::Remote)
        } else if rest.starts_with("heads/") || rest.starts_with("tags/") {
            Some(RefKind::Local)
        } else {
            None
        };
    }
    (name == "HEAD").then_some(RefKind::Local)
}

/// Watches the refs of the projects' repositories (`<git dir>`, `FETCH_HEAD` and `packed-refs` directly, `refs/**`).
/// Any change wakes the PR loop; a remote one also calls `on_remote` (the janitor: a merge may have arrived).
pub struct RefWatch {
    watcher: Option<notify::RecommendedWatcher>,
    dirs: BTreeSet<PathBuf>,
    common: HashMap<PathBuf, Option<PathBuf>>,
    on_remote: Arc<dyn Fn() + Send + Sync>,
}

impl RefWatch {
    pub fn new(on_remote: Arc<dyn Fn() + Send + Sync>) -> Self {
        RefWatch { watcher: None, dirs: BTreeSet::new(), common: HashMap::new(), on_remote }
    }

    fn common_dir(&mut self, repo: &Path) -> Option<PathBuf> {
        self.common
            .entry(repo.to_path_buf())
            .or_insert_with(|| {
                let out = axelstra_core::git::run(repo, &["rev-parse", "--git-common-dir"]).ok().filter(|o| o.ok)?;
                let p = PathBuf::from(out.stdout.trim());
                let abs = if p.is_absolute() { p } else { repo.join(p) };
                abs.is_dir().then_some(abs)
            })
            .clone()
    }

    /// Watches the repositories in `repos`; a no-op when the set is the one already watched.
    pub fn sync(&mut self, repos: &[PathBuf]) {
        let want: BTreeSet<PathBuf> = repos.iter().filter_map(|r| self.common_dir(r)).collect();
        if want == self.dirs && self.watcher.is_some() {
            return;
        }
        use notify::{RecursiveMode, Watcher};
        let on_remote = self.on_remote.clone();
        let made = notify::recommended_watcher(move |ev: notify::Result<notify::Event>| {
            let Ok(ev) = ev else { return };
            if matches!(ev.kind, notify::EventKind::Access(_)) {
                return;
            }
            let kinds: Vec<RefKind> = ev.paths.iter().filter_map(|p| classify(p)).collect();
            if kinds.is_empty() {
                return;
            }
            WAKE.bump();
            if kinds.contains(&RefKind::Remote) {
                on_remote();
            }
        });
        let Ok(mut w) = made else { return };
        for d in &want {
            let _ = w.watch(d, RecursiveMode::NonRecursive);
            let _ = w.watch(&d.join("refs"), RecursiveMode::Recursive);
        }
        self.watcher = Some(w);
        self.dirs = want;
    }

    pub fn watching(&self) -> usize {
        self.dirs.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checks::Check;

    fn st(state: &str, ci: &str, sha: &str) -> Status {
        Status { number: 7, pr_state: state.into(), ci: ci.into(), head_sha: sha.into(), ..Default::default() }
    }

    #[test]
    fn what_a_pr_state_waits_for() {
        assert_eq!(interval_ms(&st("open", "pending", "a")), Some(PENDING_MS));
        for ci in ["pass", "fail", "none"] {
            assert_eq!(interval_ms(&st("open", ci, "a")), Some(OPEN_MS));
        }
        assert_eq!(interval_ms(&st("merged", "pass", "a")), None);
        assert_eq!(interval_ms(&st("closed", "pending", "a")), None);
        let bad = Status { problem: "auth".into(), pr_state: "open".into(), ..Default::default() };
        assert_eq!(interval_ms(&bad), None);
    }

    #[test]
    fn first_same_changed() {
        let mut t = Table::default();
        let p = Path::new("C:/r/wt");
        assert_eq!(t.note(p, &st("open", "pending", "a"), 1_000), Observed::First);
        assert_eq!(t.note(p, &st("open", "pending", "a"), 2_000), Observed::Same);
        assert_eq!(t.note(p, &st("open", "pass", "a"), 3_000), Observed::Changed);
        // a check changing inside the same overall state is a change too
        let mut s = st("open", "pass", "a");
        s.checks = vec![Check { name: "build".into(), state: "pass".into(), link: String::new() }];
        assert_eq!(t.note(p, &s, 4_000), Observed::Changed);
        // a different spelling of the same path is the same worktree
        assert_eq!(t.note(Path::new("c:\\r\\wt"), &s, 5_000), Observed::Same);
        assert_eq!(t.len(), 1);
        // the PR got merged
        assert_eq!(t.note(p, &st("merged", "pass", "a"), 6_000), Observed::Changed);
    }

    #[test]
    fn the_one_deadline_is_the_earliest_due_look_and_there_is_none_without_one() {
        let mut t = Table::default();
        assert_eq!(t.next_deadline(0), None);
        t.note(Path::new("C:/a"), &st("open", "pass", "a"), 1_000);
        assert_eq!(t.next_deadline(1_000), Some(OPEN_MS));
        t.note(Path::new("C:/b"), &st("open", "pending", "b"), 10_000);
        assert_eq!(t.next_deadline(10_000), Some(PENDING_MS));
        assert_eq!(t.next_deadline(10_000 + PENDING_MS + 5), Some(0));
        // terminal ones never ask for a timer
        let mut u = Table::default();
        u.note(Path::new("C:/m"), &st("merged", "pass", "a"), 0);
        assert_eq!(u.next_deadline(1), None);
        // and a worktree nobody looked at for the interest spell is forgotten
        assert_eq!(t.next_deadline(INTEREST_MS + 20_000), None);
        t.prune(INTEREST_MS + 20_000);
        assert!(t.is_empty());
    }

    #[test]
    fn events_reread_only_old_looks_and_the_deadline_only_due_ones() {
        let mut t = Table::default();
        let p = Path::new("C:/a");
        t.note(p, &st("open", "pending", "a"), 0);
        // an event right after a look: it counts
        assert!(t.fresh(p, MIN_AGE_MS - 1, true).is_some());
        assert!(t.stale(MIN_AGE_MS - 1, true).is_empty());
        // an event later: stale
        assert!(t.fresh(p, MIN_AGE_MS, true).is_none());
        assert_eq!(t.stale(MIN_AGE_MS, true), vec![p.to_path_buf()]);
        // no event: counts until the deadline
        assert!(t.fresh(p, PENDING_MS - 1, false).is_some());
        assert!(t.stale(PENDING_MS - 1, false).is_empty());
        assert_eq!(t.stale(PENDING_MS, false), vec![p.to_path_buf()]);
        // a merged one is not re-read on every event, and never by a deadline
        let m = Path::new("C:/m");
        t.note(m, &st("merged", "pass", "a"), 0);
        assert!(t.fresh(m, 60_000, true).is_some());
        assert!(t.fresh(m, TERMINAL_AGE_MS, true).is_none());
        assert!(t.fresh(m, INTEREST_MS - 1, false).is_some());
    }

    #[test]
    fn the_table_is_bounded() {
        let mut t = Table::default();
        for i in 0..(CAP + 5) {
            t.note(Path::new(&format!("C:/w{i}")), &st("open", "pass", "a"), i as u64);
        }
        assert_eq!(t.len(), CAP);
    }

    #[test]
    fn which_git_paths_matter() {
        let c = |s: &str| classify(Path::new(s));
        assert_eq!(c("C:\\r\\.git\\refs\\remotes\\origin\\main"), Some(RefKind::Remote));
        assert_eq!(c("/r/.git/refs/heads/ax/feat"), Some(RefKind::Local));
        assert_eq!(c("/r/.git/refs/tags/v1"), Some(RefKind::Local));
        assert_eq!(c("/r/.git/FETCH_HEAD"), Some(RefKind::Fetch));
        assert_eq!(c("/r/.git/packed-refs"), Some(RefKind::Remote));
        assert_eq!(c("/r/.git/HEAD"), Some(RefKind::Local));
        assert_eq!(c("/r/.git/refs/remotes/origin/main.lock"), None);
        assert_eq!(c("/r/.git/packed-refs.lock"), None);
        assert_eq!(c("/r/.git/objects/ab/cdef"), None);
        assert_eq!(c("/r/.git/index"), None);
        assert_eq!(c("/r/.git/refs/stash"), None);
    }

    #[test]
    fn wake_ends_a_wait_and_the_timeout_ends_it_too() {
        let w = Arc::new(Wake::new());
        let seen = w.now();
        assert_eq!(w.wait(seen, Some(Duration::from_millis(20))), seen);
        let w2 = w.clone();
        let h = std::thread::spawn(move || w2.wait(seen, None));
        std::thread::sleep(Duration::from_millis(50));
        w.bump();
        assert_eq!(h.join().unwrap(), seen + 1);
    }

    /// A ref moving in a real repository wakes the loop; a remote one also calls the janitor's hook; a commit on a
    /// local branch only wakes the loop.
    #[test]
    fn a_ref_moving_in_a_repository_wakes_the_loop() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let root = std::env::temp_dir().join(format!("axl-refwatch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let g = |args: &[&str]| assert!(axelstra_core::git::run(&root, args).unwrap().ok, "git {args:?}");
        g(&["init", "-q", "-b", "main"]);
        g(&["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "--allow-empty", "-m", "x"]);
        let remote = Arc::new(AtomicUsize::new(0));
        let r2 = remote.clone();
        let mut rw = RefWatch::new(Arc::new(move || {
            r2.fetch_add(1, Ordering::SeqCst);
        }));
        rw.sync(std::slice::from_ref(&root));
        assert_eq!(rw.watching(), 1);
        // the same set again changes nothing
        rw.sync(std::slice::from_ref(&root));
        assert_eq!(rw.watching(), 1);
        let until = |what: &str, f: &dyn Fn() -> bool| {
            let t = std::time::Instant::now();
            while !f() {
                assert!(t.elapsed() < Duration::from_secs(20), "never woke: {what}");
                std::thread::sleep(Duration::from_millis(25));
            }
        };
        let before = WAKE.now();
        g(&["branch", "ax/x"]);
        until("local branch", &|| WAKE.now() > before);
        assert_eq!(remote.load(Ordering::SeqCst), 0, "a local branch is not a remote ref");
        g(&["update-ref", "refs/remotes/origin/main", "HEAD"]);
        until("remote ref", &|| remote.load(Ordering::SeqCst) > 0);
        drop(rw);
        let _ = std::fs::remove_dir_all(&root);
    }
}
