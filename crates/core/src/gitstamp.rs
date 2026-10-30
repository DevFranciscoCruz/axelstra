//! Fingerprints of a repository read from its files, so an answer git gave can be reused until git's own state moves:
//! no process is started to find out and no clock decides. A fingerprint is a hash of what git itself reads for the
//! answer: `HEAD`s, loose refs, `packed-refs`, the `worktrees/` records, the index or the config file. `None` means
//! "cannot tell" (no `.git` here, a read failed, too many refs): the caller then asks git as before and keeps nothing.

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// More loose refs than this and the walk costs more than it saves: no fingerprint.
const MAX_REFS: usize = 20_000;

/// The checkout's own git dir and the repository's common dir.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dirs {
    pub git_dir: PathBuf,
    pub common: PathBuf,
}

/// Found from `path/.git` the way git does: a folder (main checkout), or a file `gitdir: <dir>` (linked worktree)
/// whose `commondir` names the shared one. `None` when `path` has no `.git` of its own (a plain folder, a subfolder).
pub fn dirs(path: &Path) -> Option<Dirs> {
    let dot = path.join(".git");
    let meta = std::fs::metadata(&dot).ok()?;
    if meta.is_dir() {
        return Some(Dirs { git_dir: dot.clone(), common: dot });
    }
    let text = std::fs::read_to_string(&dot).ok()?;
    let gd = text.trim().strip_prefix("gitdir:")?.trim();
    let gd = if Path::new(gd).is_absolute() { PathBuf::from(gd) } else { path.join(gd) };
    let common = match std::fs::read_to_string(gd.join("commondir")) {
        Ok(c) => {
            let c = c.trim();
            if Path::new(c).is_absolute() {
                PathBuf::from(c)
            } else {
                gd.join(c)
            }
        }
        Err(_) => gd.clone(),
    };
    Some(Dirs { git_dir: gd, common })
}

/// Length and modification time: what changes when a file is rewritten.
fn meta(h: &mut DefaultHasher, p: &Path) -> bool {
    match std::fs::metadata(p) {
        Ok(m) => {
            m.len().hash(h);
            m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_nanos()).hash(h);
            true
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            0u8.hash(h);
            true
        }
        Err(_) => false,
    }
}

/// The content of a small file (a ref, a `HEAD`), or a marker when it does not exist.
fn content(h: &mut DefaultHasher, p: &Path) -> bool {
    match std::fs::read(p) {
        Ok(b) => {
            b.hash(h);
            true
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            0u8.hash(h);
            true
        }
        Err(_) => false,
    }
}

/// Every loose ref under `dir` (path and content), in a stable order.
fn walk_refs(h: &mut DefaultHasher, dir: &Path, count: &mut usize) -> bool {
    let rd = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(e) => return e.kind() == std::io::ErrorKind::NotFound,
    };
    let mut entries: Vec<(String, bool)> = Vec::new();
    for e in rd {
        let Ok(e) = e else { return false };
        let Ok(ft) = e.file_type() else { return false };
        entries.push((e.file_name().to_string_lossy().into_owned(), ft.is_dir()));
    }
    entries.sort();
    for (name, is_dir) in entries {
        *count += 1;
        if *count > MAX_REFS {
            return false;
        }
        name.hash(h);
        let p = dir.join(&name);
        let ok = if is_dir { walk_refs(h, &p, count) } else { content(h, &p) };
        if !ok {
            return false;
        }
    }
    true
}

/// Moves whenever a branch, a remote-tracking ref, a tag, the `HEAD` of any of the repo's checkouts or the set of its
/// worktrees changes: everything `worktree list`, `rev-parse <ref>`, `merge-base`, `rev-list`, `cherry` or `diff A...B`
/// read. Works from any checkout of the repo (main or linked).
pub fn refs(path: &Path) -> Option<u64> {
    let d = dirs(path)?;
    let mut h = DefaultHasher::new();
    let mut ok = content(&mut h, &d.common.join("HEAD")) && content(&mut h, &d.git_dir.join("HEAD"));
    ok = ok && meta(&mut h, &d.common.join("packed-refs"));
    // the reftable backend keeps every ref in its tables, listed in `tables.list`
    ok = ok && content(&mut h, &d.common.join("reftable").join("tables.list"));
    let mut count = 0;
    ok = ok && walk_refs(&mut h, &d.common.join("refs"), &mut count);
    // the linked worktrees: where each one is, what it has checked out, whether it is locked
    match std::fs::read_dir(d.common.join("worktrees")) {
        Ok(rd) => {
            let mut names: Vec<String> = rd.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).collect();
            names.sort();
            for n in names {
                let w = d.common.join("worktrees").join(&n);
                n.hash(&mut h);
                ok = ok && content(&mut h, &w.join("HEAD")) && content(&mut h, &w.join("gitdir")) && content(&mut h, &w.join("locked"));
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => ok = false,
    }
    ok.then(|| h.finish())
}

/// Moves with the checkout's index: staging, a commit, a checkout, a merge or a `git status` that refreshed it.
pub fn index(path: &Path) -> Option<u64> {
    let d = dirs(path)?;
    let mut h = DefaultHasher::new();
    meta(&mut h, &d.git_dir.join("index")).then(|| h.finish())
}

/// Moves with the repository's config file (remotes, their urls).
pub fn config(path: &Path) -> Option<u64> {
    let d = dirs(path)?;
    let mut h = DefaultHasher::new();
    meta(&mut h, &d.common.join("config")).then(|| h.finish())
}

/// Several fingerprints as one.
pub fn combine(parts: &[u64]) -> u64 {
    let mut h = DefaultHasher::new();
    parts.hash(&mut h);
    h.finish()
}

/// Entries kept by one [`Stamped`]; past it the table starts over (the next lookups ask git again).
const MAX_ENTRIES: usize = 4096;

/// Answers kept under a key while their fingerprint holds. A lookup with no fingerprint computes and keeps nothing.
pub struct Stamped<T> {
    map: Mutex<Option<Kept<T>>>,
}

/// key -> (fingerprint, when computed, answer)
type Kept<T> = HashMap<String, (u64, Instant, T)>;

impl<T: Clone> Default for Stamped<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Clone> Stamped<T> {
    pub const fn new() -> Self {
        Stamped { map: Mutex::new(None) }
    }

    /// The kept answer of `key` while `stamp` is the one it was computed under (and, with `max_age`, younger than
    /// that: for what no fingerprint covers, like files edited in a checkout), else `f()`, kept. `stamp` must be read
    /// BEFORE the work it vouches for, so a change during the work leaves an entry the next lookup does not match.
    pub fn get(&self, key: &str, stamp: Option<u64>, max_age: Option<Duration>, f: impl FnOnce() -> T) -> T {
        let Some(stamp) = stamp else { return f() };
        if let Some((s, at, v)) = self.map.lock().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|m| m.get(key)) {
            if *s == stamp && max_age.is_none_or(|a| at.elapsed() < a) {
                return v.clone();
            }
        }
        let v = f();
        let mut g = self.map.lock().unwrap_or_else(|e| e.into_inner());
        let m = g.get_or_insert_with(HashMap::new);
        if m.len() >= MAX_ENTRIES {
            m.clear();
        }
        m.insert(key.to_string(), (stamp, Instant::now(), v.clone()));
        v
    }

    /// Drops everything kept (a change the fingerprints may not see was made here).
    pub fn clear(&self) {
        *self.map.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn git(dir: &Path, args: &[&str]) {
        let o = Command::new("git").current_dir(dir).args(["-c", "user.name=t", "-c", "user.email=t@t"]).args(args).output().unwrap();
        assert!(o.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&o.stderr));
    }

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("axl-stamp-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn the_ref_fingerprint_moves_with_commits_branches_worktrees_and_packing() {
        let dir = temp("refs");
        let repo = dir.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        assert_eq!(refs(&repo), None, "a plain folder has no fingerprint");
        git(&repo, &["init", "-q", "-b", "main"]);
        git(&repo, &["commit", "-q", "--allow-empty", "-m", "a"]);
        let a = refs(&repo).unwrap();
        assert_eq!(refs(&repo), Some(a), "nothing moved");
        git(&repo, &["commit", "-q", "--allow-empty", "-m", "b"]);
        let b = refs(&repo).unwrap();
        assert_ne!(a, b, "a commit moves the branch");
        git(&repo, &["branch", "ax/x"]);
        let c = refs(&repo).unwrap();
        assert_ne!(b, c, "a new branch");
        let wt = dir.join("wt");
        git(&repo, &["worktree", "add", "-q", wt.to_str().unwrap(), "ax/x"]);
        let d = refs(&repo).unwrap();
        assert_ne!(c, d, "a new worktree");
        let from_wt = refs(&wt).unwrap();
        git(&wt, &["commit", "-q", "--allow-empty", "-m", "c"]);
        let e = refs(&repo).unwrap();
        assert_ne!(d, e, "a commit in the linked worktree");
        assert_ne!(refs(&wt), Some(from_wt), "seen from the linked worktree too");
        git(&repo, &["pack-refs", "--all"]);
        assert_ne!(refs(&repo), Some(e), "packing rewrites packed-refs");
        git(&wt, &["checkout", "-q", "--detach"]);
        let f = refs(&repo).unwrap();
        git(&wt, &["checkout", "-q", "ax/x"]);
        assert_ne!(refs(&repo), Some(f), "the worktree's HEAD moved");
        let i = index(&wt).unwrap();
        std::fs::write(wt.join("f.txt"), "x").unwrap();
        git(&wt, &["add", "f.txt"]);
        assert_ne!(index(&wt), Some(i), "staging rewrites the index");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_kept_answer_holds_while_its_fingerprint_does() {
        let s: Stamped<u32> = Stamped::new();
        let mut runs = 0;
        let mut ask = |stamp, age| {
            s.get("k", stamp, age, || {
                runs += 1;
                runs
            })
        };
        assert_eq!(ask(Some(1), None), 1);
        assert_eq!(ask(Some(1), None), 1);
        assert_eq!(ask(Some(2), None), 2);
        assert_eq!(ask(None, None), 3, "no fingerprint: computed every time");
        assert_eq!(ask(Some(2), None), 2, "and nothing kept for it");
        assert_eq!(ask(Some(2), Some(Duration::ZERO)), 4, "older than max_age");
    }
}
