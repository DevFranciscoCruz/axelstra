//! Tells a client when a worktree changed, so the UI asks git only when there is something to ask.
//! One native watcher per subscription, events filtered like `git status` would see them (nested `.gitignore`
//! files included) and debounced; the work is on a plain thread, the daemon core stays async.
//!
//! The OS is asked to watch only the folders `git` would not ignore, one non-recursive watch each: a recursive
//! watch on the root would also cover `node_modules`/`target`/`dist` (thousands of folders, the inotify limit on
//! Linux, a heavy handle load on Windows) and could only be filtered after the OS had already reported the event.

use anyhow::{Context, Result};
use axelstra_core::git;
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use notify::{RecursiveMode, Watcher};
use std::collections::HashMap;
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

/// Folders one subscription may watch; past this the rest is skipped (a monorepo of that size gets polled changes
/// for what is watched and a missed event elsewhere, instead of exhausting the OS limit for every program).
const MAX_WATCHES: usize = 10_000;
/// Events queued between the OS callback and the debouncing thread; a burst past it is folded into one "something changed".
const QUEUE: usize = 4096;
/// How often an idle watcher checks whether it was told to stop (nothing else wakes it).
const STOP_CHECK: Duration = Duration::from_secs(5);

/// The `.gitignore` files of a worktree, each applied to its own folder and below, as git does, plus
/// `.git/info/exclude` for the root. Loaded lazily and kept until a `.gitignore` changes.
pub struct Ignores {
    root: PathBuf,
    exclude: PathBuf,
    cache: HashMap<PathBuf, Gitignore>,
}

impl Ignores {
    pub fn new(root: &Path, git_dir: &Path) -> Self {
        Ignores { root: root.to_path_buf(), exclude: git_dir.join("info").join("exclude"), cache: HashMap::new() }
    }

    /// Forget what was read: a `.gitignore` was edited.
    pub fn reload(&mut self) {
        self.cache.clear();
    }

    fn of(&mut self, dir: &Path) -> &Gitignore {
        let (root, exclude) = (&self.root, &self.exclude);
        self.cache.entry(dir.to_path_buf()).or_insert_with(|| {
            let mut b = GitignoreBuilder::new(dir);
            b.add(dir.join(".gitignore"));
            if dir == root.as_path() {
                b.add(exclude);
            }
            b.build().unwrap_or_else(|_| Gitignore::empty())
        })
    }

    /// True when git would ignore `path` (under the root): some folder on the way, or the entry itself, matches a
    /// rule of the `.gitignore` in its own folder or above (the deepest file with a verdict wins, so `!keep` works).
    pub fn is_ignored(&mut self, path: &Path, is_dir: bool) -> bool {
        let Ok(rel) = path.strip_prefix(&self.root) else { return false };
        let parts: Vec<_> = rel.components().collect();
        let mut dir = self.root.clone();
        for (i, c) in parts.iter().enumerate() {
            let entry = dir.join(c.as_os_str());
            let last = i + 1 == parts.len();
            let entry_is_dir = !last || is_dir;
            // the files that apply to `entry`: the .gitignore of every folder from `dir` up to the root
            let mut verdict = None;
            let mut d = dir.clone();
            loop {
                let m = self.of(&d).matched(&entry, entry_is_dir);
                if !m.is_none() {
                    verdict = Some(m.is_ignore());
                    break;
                }
                if d == self.root || !d.pop() {
                    break;
                }
            }
            if verdict == Some(true) {
                return true;
            }
            dir = entry;
        }
        false
    }
}

/// Classifies an event path. `git_dir` is the (possibly shared) git directory of the worktree.
pub fn classify(path: &Path, root: &Path, git_dir: &Path, ignore: &mut Ignores) -> Kind {
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
    // a path that is gone cannot be asked whether it was a folder: a rule for folders only (`build/`) is then
    // applied through the parents, which is what matters for everything inside it
    if ignore.is_ignored(path, path.is_dir()) {
        return Kind::Ignore;
    }
    Kind::Work
}

/// The folders to watch under `root`: every folder git does not ignore, `.git` and the noisy ones left out,
/// the root itself first. Capped at `MAX_WATCHES`.
pub fn watch_dirs(root: &Path) -> Vec<PathBuf> {
    let walk = ignore::WalkBuilder::new(root)
        .hidden(false)
        .require_git(false)
        .git_global(false)
        .filter_entry(|e| {
            let n = e.file_name().to_string_lossy();
            n != ".git" && !NOISY_DIRS.contains(&n.as_ref())
        })
        .build();
    walk.flatten().filter(|e| e.file_type().is_some_and(|t| t.is_dir())).map(|e| e.into_path()).take(MAX_WATCHES).collect()
}

/// The git directory's own files (HEAD, index, refs): `objects` and `logs` are not watched, their churn is ignored anyway.
fn git_watch_dirs(gdir: &Path) -> Vec<PathBuf> {
    let mut v = vec![gdir.to_path_buf()];
    let refs = gdir.join("refs");
    if refs.is_dir() {
        let walk = ignore::WalkBuilder::new(&refs).standard_filters(false).build();
        v.extend(walk.flatten().filter(|e| e.file_type().is_some_and(|t| t.is_dir())).map(|e| e.into_path()).take(2048));
    }
    v
}

fn git_dir(root: &Path) -> Result<PathBuf> {
    let d = git::must(root, &["rev-parse", "--absolute-git-dir"])?;
    Ok(PathBuf::from(d))
}

/// Starts watching `root`; `emit` runs (on the watcher thread) once per debounced batch.
/// The thread ends when `stop` is set.
pub fn start(root: PathBuf, stop: Arc<AtomicBool>, emit: impl Fn() + Send + 'static) -> Result<()> {
    // FSEvents reports real paths (`/private/var/...` for a `/var/...` folder), so events would never sit under a
    // symlinked root; Windows keeps its path as given (canonicalize would add the `\?\` prefix)
    #[cfg(target_os = "macos")]
    let root = root.canonicalize().unwrap_or(root);
    let gdir = git_dir(&root).context("find git dir")?;
    #[cfg(target_os = "macos")]
    let gdir = gdir.canonicalize().unwrap_or(gdir);
    let mut ignore = Ignores::new(&root, &gdir);

    // bounded: a runaway writer cannot grow the queue without limit; what does not fit is remembered as one flag
    let (tx, rx) = mpsc::sync_channel(QUEUE);
    let overflow = Arc::new(AtomicBool::new(false));
    let of = overflow.clone();
    let mut w = notify::recommended_watcher(move |ev: notify::Result<notify::Event>| match ev {
        Ok(ev) => {
            if tx.try_send(ev).is_err() {
                of.store(true, Ordering::SeqCst);
            }
        }
        // the OS dropped events (inotify queue overflow, ReadDirectoryChanges buffer): rescan on the client side;
        // the empty event wakes the thread below to say so
        Err(_) => {
            of.store(true, Ordering::SeqCst);
            let _ = tx.try_send(notify::Event::new(notify::EventKind::Other));
        }
    })
    .map_err(|e| axelstra_core::coded!("watch.create_failed", "create watcher: {detail}", detail = e))?;
    let mut watched = std::collections::HashSet::new();
    let dirs = watch_dirs(&root);
    if dirs.is_empty() {
        return Err(axelstra_core::coded!("watch.nothing", "nothing to watch in {path}", path = root.display()));
    }
    for d in dirs {
        if w.watch(&d, RecursiveMode::NonRecursive).is_ok() {
            watched.insert(d);
        }
    }
    if !watched.contains(&root) {
        return Err(axelstra_core::coded!("watch.failed", "could not watch the worktree"));
    }
    // HEAD/index live in the git directory, which for a linked worktree is outside its own folder
    for d in git_watch_dirs(&gdir) {
        if w.watch(&d, RecursiveMode::NonRecursive).is_ok() {
            watched.insert(d);
        }
    }

    std::thread::spawn(move || {
        let mut w = w; // the watcher lives as long as this thread
        let mut due: Option<Instant> = None;
        loop {
            if stop.load(Ordering::SeqCst) {
                return;
            }
            // asleep until the OS reports a change or the flush is due; `stop` has no wake-up of its own, so an idle
            // watcher looks at it every STOP_CHECK
            let wait = due.map(|d| d.saturating_duration_since(Instant::now())).unwrap_or(STOP_CHECK);
            match rx.recv_timeout(wait.min(STOP_CHECK)) {
                Ok(ev) => {
                    if matches!(ev.kind, notify::EventKind::Access(_)) {
                        continue;
                    }
                    for p in &ev.paths {
                        if p.file_name().is_some_and(|n| n == ".gitignore") {
                            ignore.reload();
                        }
                        let kind = classify(p, &root, &gdir, &mut ignore);
                        // a folder that appeared (or moved in) and is not ignored gets watched too, with its subfolders
                        if kind == Kind::Work
                            && matches!(
                                ev.kind,
                                notify::EventKind::Create(_) | notify::EventKind::Modify(notify::event::ModifyKind::Name(_))
                            )
                            && p.is_dir()
                        {
                            for d in watch_dirs(p) {
                                if watched.len() < MAX_WATCHES
                                    && !watched.contains(&d)
                                    && !ignore.is_ignored(&d, true)
                                    && w.watch(&d, RecursiveMode::NonRecursive).is_ok()
                                {
                                    watched.insert(d);
                                }
                            }
                        }
                        let delay = match kind {
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
            if overflow.swap(false, Ordering::SeqCst) {
                let at = Instant::now() + WORK_DEBOUNCE;
                due = Some(due.map_or(at, |d| d.min(at)));
            }
            if due.is_some_and(|d| Instant::now() >= d) {
                due = None;
                emit();
            }
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("axl-watch-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn put(p: &Path, text: &str) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    #[test]
    fn nested_gitignore_files_apply_to_their_own_folder_only() {
        let d = tmpdir("nested");
        let g = d.join(".git");
        std::fs::create_dir_all(&g).unwrap();
        put(&d.join(".gitignore"), "/out/\n*.log\n");
        put(&d.join("pkg/.gitignore"), "*.gen\ntmp/\n!keep.log\n");
        put(&d.join("pkg/a.rs"), "");
        put(&d.join("other/a.gen"), "");
        let mut ig = Ignores::new(&d, &g);
        let kind = |ig: &mut Ignores, rel: &str| classify(&d.join(rel), &d, &g, ig);
        // the root file
        assert_eq!(kind(&mut ig, "x.log"), Kind::Ignore);
        assert_eq!(kind(&mut ig, "out/deep/f.rs"), Kind::Ignore);
        assert_eq!(kind(&mut ig, "src/main.rs"), Kind::Work);
        // pkg/.gitignore: its rules reach pkg and below, not siblings
        assert_eq!(kind(&mut ig, "pkg/x.gen"), Kind::Ignore);
        assert_eq!(kind(&mut ig, "pkg/sub/y.gen"), Kind::Ignore);
        assert_eq!(kind(&mut ig, "pkg/tmp/z.rs"), Kind::Ignore);
        assert_eq!(kind(&mut ig, "pkg/a.rs"), Kind::Work);
        assert_eq!(kind(&mut ig, "other/a.gen"), Kind::Work, "a nested rule must not leak to a sibling folder");
        // the deeper file wins over the root: `!keep.log` re-includes what `*.log` ignored
        assert_eq!(kind(&mut ig, "pkg/keep.log"), Kind::Work);
        assert_eq!(kind(&mut ig, "pkg/other.log"), Kind::Ignore);
        // an edit of a .gitignore is picked up after a reload
        put(&d.join("other/.gitignore"), "*.gen\n");
        assert_eq!(kind(&mut ig, "other/a.gen"), Kind::Work);
        ig.reload();
        assert_eq!(kind(&mut ig, "other/a.gen"), Kind::Ignore);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn only_folders_git_would_look_at_are_watched() {
        let d = tmpdir("dirs");
        std::fs::create_dir_all(d.join(".git/objects")).unwrap();
        put(&d.join(".gitignore"), "/gen/\n");
        put(&d.join("pkg/.gitignore"), "cache/\n");
        for dir in ["src/deep", "gen/inner", "node_modules/x/y", "target/debug", "pkg/cache/z", "pkg/lib"] {
            std::fs::create_dir_all(d.join(dir)).unwrap();
        }
        let got: Vec<String> = watch_dirs(&d).iter().map(|p| p.strip_prefix(&d).unwrap().to_string_lossy().replace('\\', "/")).collect();
        for want in ["", "src", "src/deep", "pkg", "pkg/lib"] {
            assert!(got.iter().any(|g| g == want), "{want:?} missing from {got:?}");
        }
        for not in ["gen", "gen/inner", "node_modules", "node_modules/x", "target", "pkg/cache", "pkg/cache/z", ".git", ".git/objects"] {
            assert!(!got.iter().any(|g| g == not), "{not:?} must not be watched: {got:?}");
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn git_events_keep_their_own_kind() {
        let d = tmpdir("git");
        let g = d.join(".git");
        std::fs::create_dir_all(&g).unwrap();
        let mut ig = Ignores::new(&d, &g);
        assert_eq!(classify(&g.join("HEAD"), &d, &g, &mut ig), Kind::Git);
        assert_eq!(classify(&g.join("index.lock"), &d, &g, &mut ig), Kind::Ignore);
        assert_eq!(classify(&g.join("objects/ab/cd"), &d, &g, &mut ig), Kind::Ignore);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_live_watch_sees_new_folders_and_stays_quiet_about_ignored_ones() {
        use std::sync::atomic::AtomicUsize;
        let d = tmpdir("live");
        assert!(git::run(&d, &["init", "-q"]).unwrap().ok);
        put(
            &d.join(".gitignore"),
            "ignored/
",
        );
        std::fs::create_dir_all(d.join("src")).unwrap();
        std::fs::create_dir_all(d.join("ignored")).unwrap();
        std::fs::create_dir_all(d.join("node_modules")).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let n = Arc::new(AtomicUsize::new(0));
        let n2 = n.clone();
        start(d.clone(), stop.clone(), move || {
            n2.fetch_add(1, Ordering::SeqCst);
        })
        .unwrap();
        let settle = |ms: u64| std::thread::sleep(Duration::from_millis(ms));
        let wait_for = |want: usize| {
            for _ in 0..60 {
                if n.load(Ordering::SeqCst) >= want {
                    return true;
                }
                settle(100);
            }
            false
        };
        settle(300);
        put(&d.join("ignored/a.txt"), "x");
        put(&d.join("node_modules/b.js"), "x");
        settle(1500);
        assert_eq!(n.load(Ordering::SeqCst), 0, "ignored folders must not wake the client");
        put(&d.join("src/a.rs"), "x");
        assert!(wait_for(1), "a change in a watched folder");
        // a folder created after the start is watched from then on
        std::fs::create_dir_all(d.join("src/new")).unwrap();
        assert!(wait_for(2), "the new folder");
        settle(800);
        put(&d.join("src/new/f.rs"), "x");
        assert!(wait_for(3), "a file inside the new folder");
        stop.store(true, Ordering::SeqCst);
        settle(700);
        let _ = std::fs::remove_dir_all(&d);
    }
}
