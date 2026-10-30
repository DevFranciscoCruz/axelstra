//! `dispatch.overlaps`: which open worktrees edit the same files. Same idea as `overlaps_with` in
//! `axelstra dispatch list`, but as a daemon method so the sidebar can show it. Computing it runs git in every
//! worktree, so the answer is cached for a few seconds.

use crate::store::LockRecover;
use axelstra_core::{git, gitstamp};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const TTL: Duration = Duration::from_secs(10);

type Snapshot = Vec<(String, BTreeSet<String>)>;
static CACHE: Mutex<Option<(Instant, Snapshot)>> = Mutex::new(None);

/// Key of `dispatch.json`, which the app writes lowercase on every platform.
fn norm(p: &str) -> String {
    axelstra_core::paths::norm(p).to_lowercase()
}

fn listed(wt: &Path, args: &[&str], out: &mut BTreeSet<String>) {
    if let Ok(o) = git::run(wt, args) {
        if o.ok {
            out.extend(o.stdout.lines().map(|l| l.trim().replace('\\', "/")).filter(|l| !l.is_empty()));
        }
    }
}

/// The two halves of `changed_files`, each kept while what it reads holds (see `gitstamp`, `memo::worktree_keep`).
static COMMITTED: gitstamp::Stamped<BTreeSet<String>> = gitstamp::Stamped::new();
static UNCOMMITTED: gitstamp::Stamped<BTreeSet<String>> = gitstamp::Stamped::new();

/// Files a worktree changed: committed since the default branch plus uncommitted.
fn changed_files(wt: &Path, base: &str) -> BTreeSet<String> {
    let key = axelstra_core::paths::norm(&wt.to_string_lossy());
    // committed: commits only, so this checkout's HEAD and the base branch say when to look again (a commit or a
    // branch elsewhere in the repo does not)
    let stamp = (|| Some(gitstamp::combine(&[gitstamp::head(wt)?, gitstamp::tips(wt, &[base])?])))();
    let mut out = COMMITTED.get(&format!("{key}|{base}"), stamp, None, || {
        let mut s = BTreeSet::new();
        listed(wt, &["diff", "--name-only", &format!("{base}...HEAD")], &mut s);
        s
    });
    // uncommitted: tracked edits (staged or not, renames included) and new files. `git::run` trims the output,
    // which would eat the leading space of the first `status --porcelain` line, so the two listings are asked for apart.
    // Kept until the daemon's watch of this worktree fires or its index, HEAD or branch move (an unwatched one: also the age floor)
    let k = crate::memo::worktree_keep(&wt.to_string_lossy());
    out.extend(UNCOMMITTED.get(&key, k.stamp, k.max_age, || {
        let mut s = BTreeSet::new();
        listed(wt, &["diff", "--name-only", "HEAD"], &mut s);
        listed(wt, &["ls-files", "--others", "--exclude-standard"], &mut s);
        s
    }));
    out
}

/// Pairs of worktrees (paths as git lists them) that touch a common file. Worktrees with nothing in common are left out.
pub fn invert(files: &[(String, BTreeSet<String>)]) -> BTreeMap<String, Vec<String>> {
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (path, mine) in files {
        let others: Vec<String> =
            files.iter().filter(|(o, theirs)| o != path && theirs.intersection(mine).next().is_some()).map(|(o, _)| o.clone()).collect();
        if !others.is_empty() {
            out.insert(path.clone(), others);
        }
    }
    out
}

/// For every worktree, the files it shares with each other one: `{path: [{"with": other path, "files": [...]}]}`.
/// Worktrees without a shared file are absent.
pub fn shared_files(files: &[(String, BTreeSet<String>)]) -> BTreeMap<String, Vec<(String, Vec<String>)>> {
    let mut out: BTreeMap<String, Vec<(String, Vec<String>)>> = BTreeMap::new();
    for (path, mine) in files {
        let list: Vec<(String, Vec<String>)> = files
            .iter()
            .filter(|(o, _)| o != path)
            .filter_map(|(o, theirs)| {
                let common: Vec<String> = theirs.intersection(mine).cloned().collect();
                (!common.is_empty()).then(|| (o.clone(), common))
            })
            .collect();
        if !list.is_empty() {
            out.insert(path.clone(), list);
        }
    }
    out
}

fn compute(store: &Path, dispatch_json: &Path) -> Vec<(String, BTreeSet<String>)> {
    let meta: Value = std::fs::read_to_string(dispatch_json).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_else(|| json!({}));
    let mut files: Vec<(String, BTreeSet<String>)> = Vec::new();
    for p in crate::projects::load(store) {
        let repo = Path::new(&p.path);
        let Ok(list) = git::worktree_list(repo) else { continue };
        let base = git::default_branch(repo);
        for w in list.into_iter().filter(|w| !w.is_main) {
            // archived worktrees are done with: they do not conflict with anything
            if meta[norm(&w.path)]["archived"] == json!(true) {
                continue;
            }
            let set = changed_files(Path::new(&w.path), &base);
            files.push((w.path, set));
        }
    }
    files
}

/// The changed files of every open worktree, cached for `TTL`: both overlap methods read the same snapshot.
fn snapshot(store: &Path, dispatch_json: &Path) -> Vec<(String, BTreeSet<String>)> {
    if let Some((at, v)) = CACHE.lock_recover().as_ref() {
        if at.elapsed() < TTL {
            return v.clone();
        }
    }
    let v = compute(store, dispatch_json);
    *CACHE.lock_recover() = Some((Instant::now(), v.clone()));
    v
}

/// Drops the snapshot: a rebase just changed what the worktrees touch. The per-worktree halves stay: the rebased
/// checkout's HEAD and index moved, so its stamps no longer match, and the others did not change.
pub fn forget() {
    *CACHE.lock_recover() = None;
}

/// `{worktree path: [other worktree paths touching the same files]}`; worktrees without overlaps are absent.
pub fn overlaps(store: &Path, dispatch_json: &Path) -> Value {
    let mut m = Map::new();
    for (path, others) in invert(&snapshot(store, dispatch_json)) {
        m.insert(path, json!(others));
    }
    Value::Object(m)
}

/// `{worktree path: [{"with": other worktree path, "files": [shared files]}]}`; worktrees without overlaps are absent.
pub fn overlap_files(store: &Path, dispatch_json: &Path) -> Value {
    let mut m = Map::new();
    for (path, list) in shared_files(&snapshot(store, dispatch_json)) {
        m.insert(path, json!(list.into_iter().map(|(with, files)| json!({"with": with, "files": files})).collect::<Vec<_>>()));
    }
    Value::Object(m)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(v: &[&str]) -> BTreeSet<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn worktrees_sharing_a_file_point_at_each_other() {
        let files = vec![
            ("a".to_string(), set(&["src/x.rs", "README.md"])),
            ("b".to_string(), set(&["src/x.rs"])),
            ("c".to_string(), set(&["src/y.rs"])),
            ("d".to_string(), set(&[])),
        ];
        let m = invert(&files);
        assert_eq!(m.get("a"), Some(&vec!["b".to_string()]));
        assert_eq!(m.get("b"), Some(&vec!["a".to_string()]));
        assert!(!m.contains_key("c") && !m.contains_key("d"));
    }

    #[test]
    fn shared_files_lists_the_common_files_per_pair() {
        let files = vec![
            ("a".to_string(), set(&["src/x.rs", "README.md", "z"])),
            ("b".to_string(), set(&["src/x.rs", "z"])),
            ("c".to_string(), set(&["src/y.rs"])),
        ];
        let m = shared_files(&files);
        assert_eq!(m["a"], vec![("b".to_string(), vec!["src/x.rs".to_string(), "z".to_string()])]);
        assert_eq!(m["b"], vec![("a".to_string(), vec!["src/x.rs".to_string(), "z".to_string()])]);
        assert!(!m.contains_key("c"));
    }

    #[test]
    fn changed_files_sees_committed_and_uncommitted_edits() {
        let r = std::env::temp_dir().join(format!("axl-overlap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&r);
        std::fs::create_dir_all(&r).unwrap();
        git::must(&r, &["init", "-q", "-b", "main"]).unwrap();
        let cfg = ["-c", "user.name=t", "-c", "user.email=t@t"];
        std::fs::write(r.join("a.txt"), "1").unwrap();
        git::must(&r, &[cfg[0], cfg[1], cfg[2], cfg[3], "add", "."]).unwrap();
        git::must(&r, &[cfg[0], cfg[1], cfg[2], cfg[3], "commit", "-qm", "init"]).unwrap();
        git::must(&r, &["checkout", "-qb", "topic"]).unwrap();
        std::fs::write(r.join("b.txt"), "2").unwrap();
        git::must(&r, &[cfg[0], cfg[1], cfg[2], cfg[3], "add", "."]).unwrap();
        git::must(&r, &[cfg[0], cfg[1], cfg[2], cfg[3], "commit", "-qm", "b"]).unwrap();
        std::fs::write(r.join("a.txt"), "changed").unwrap();
        assert_eq!(changed_files(&r, "main"), set(&["a.txt", "b.txt"]));
        let _ = std::fs::remove_dir_all(&r);
    }

    #[test]
    fn only_the_worktree_that_changed_runs_git_again() {
        use crate::memo::testrepo::{cleanup, edit_and_wait, git, git_spawns, with_worktrees};
        let (root, wts) = with_worktrees("overlap-spawns", 5);
        let all = || wts.iter().for_each(|w| drop(changed_files(w, "main")));
        all();
        assert_eq!(git_spawns(all), 0, "nothing changed: every answer is kept");
        // one file edited in one worktree: its uncommitted half asks git again (diff + ls-files), nobody else's does
        edit_and_wait(&wts[2], "own2.txt");
        assert_eq!(git_spawns(all), 2);
        assert_eq!(git_spawns(all), 0);
        // a commit in one worktree moves the repo's refs: only that worktree's halves are read again
        std::fs::write(wts[0].join("c.txt"), "c").unwrap();
        edit_and_wait(&wts[0], "c.txt");
        git(&wts[0], &["add", "."]);
        git(&wts[0], &["commit", "-qm", "c"]);
        let n = git_spawns(all);
        assert_eq!(n, 3, "committed diff + uncommitted diff + ls-files of the one worktree");
        assert_eq!(changed_files(&wts[0], "main"), set(&["own0.txt", "c.txt"]));
        // the default branch moved: every worktree's committed half is outdated, the uncommitted ones are not
        git(&root.join("repo"), &["commit", "-q", "--allow-empty", "-m", "m"]);
        assert_eq!(git_spawns(all), 5);
        cleanup(&root, &wts);
    }
}
