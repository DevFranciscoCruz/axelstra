//! Tells a client when a worktree changed, so the UI asks git only when there is something to ask.
//! One native watcher per subscription, events filtered like `git status` would see them and
//! debounced; the work is on a plain thread, the daemon core stays async.

use anyhow::{Context, Result};
use axelstra_core::git;
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use notify::{RecursiveMode, Watcher};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

/// Flush quickly when files in the worktree changed: latency matters.
pub const WORK_DEBOUNCE: Duration = Duration::from_millis(300);
/// Metadata-only bursts (fetch, commit bookkeeping) are coalesced harder.
pub const GIT_DEBOUNCE: Duration = Duration::from_millis(1000);

/// Directories whose contents never show up in a change list and churn constantly.
const NOISY_DIRS: &[&str] = &["node_modules", "target", "dist", "build", ".next", ".turbo", ".cache", ".kanban", ".axelstra"];

/// `.git/` top-level entries whose churn never changes `git status`.
const GIT_IGNORED_TOP: &[&str] = &["objects", "logs", "lfs"];

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Kind {
    Work,
    Git,
    Ignore,
}

/// Classifies an event path. `git_dir` is the (possibly shared) git directory of the worktree.
pub fn classify(path: &Path, root: &Path, git_dir: &Path, ignore: &Gitignore) -> Kind {
    if let Ok(rel) = path.strip_prefix(git_dir) {
        let mut parts = rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned());
        let top = parts.next().unwrap_or_default();
        let name = rel.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if GIT_IGNORED_TOP.contains(&top.as_str()) || name == "FETCH_HEAD" || name.ends_with(".lock") {
            return Kind::Ignore;
        }
        return Kind::Git;
    }
    let Ok(rel) = path.strip_prefix(root) else { return Kind::Ignore };
    if rel.components().any(|c| NOISY_DIRS.contains(&c.as_os_str().to_string_lossy().as_ref())) {
        return Kind::Ignore;
    }
    if ignore.matched_path_or_any_parents(rel, false).is_ignore() {
        return Kind::Ignore;
    }
    Kind::Work
}

fn git_dir(root: &Path) -> Result<PathBuf> {
    let d = git::must(root, &["rev-parse", "--absolute-git-dir"])?;
    Ok(PathBuf::from(d))
}

/// Starts watching `root`; `emit` runs (on the watcher thread) once per debounced batch.
/// The thread ends when `stop` is set.
pub fn start(root: PathBuf, stop: Arc<AtomicBool>, emit: impl Fn() + Send + 'static) -> Result<()> {
    let gdir = git_dir(&root).context("find git dir")?;
    let mut gi = GitignoreBuilder::new(&root);
    gi.add(root.join(".gitignore"));
    gi.add(gdir.join("info").join("exclude"));
    let ignore = gi.build().context("read .gitignore")?;

    let (tx, rx) = mpsc::channel();
    let mut w = notify::recommended_watcher(move |ev: notify::Result<notify::Event>| {
        if let Ok(ev) = ev {
            let _ = tx.send(ev);
        }
    })
    .context("create watcher")?;
    w.watch(&root, RecursiveMode::Recursive).context("watch worktree")?;
    // a linked worktree keeps HEAD/index outside its own folder
    if !gdir.starts_with(&root) {
        w.watch(&gdir, RecursiveMode::Recursive).context("watch git dir")?;
    }

    std::thread::spawn(move || {
        let _w = w; // the watcher lives as long as this thread
        let mut due: Option<Instant> = None;
        loop {
            if stop.load(Ordering::SeqCst) {
                return;
            }
            let wait = due.map(|d| d.saturating_duration_since(Instant::now())).unwrap_or(Duration::from_millis(500));
            match rx.recv_timeout(wait.min(Duration::from_millis(500))) {
                Ok(ev) => {
                    if matches!(ev.kind, notify::EventKind::Access(_)) {
                        continue;
                    }
                    for p in &ev.paths {
                        let delay = match classify(p, &root, &gdir, &ignore) {
                            Kind::Work => WORK_DEBOUNCE,
                            Kind::Git => GIT_DEBOUNCE,
                            Kind::Ignore => continue,
                        };
                        // leading-anchored: an endless burst cannot push the flush back forever
                        let at = Instant::now() + delay;
                        due = Some(due.map_or(at, |d| d.min(at)));
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            }
            if due.is_some_and(|d| Instant::now() >= d) {
                due = None;
                emit();
            }
        }
    });
    Ok(())
}
