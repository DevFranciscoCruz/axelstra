//! `dispatch.overlaps`: which open worktrees edit the same files. Same idea as `overlaps_with` in
//! `axelstra dispatch list`, but as a daemon method so the sidebar can show it. Computing it runs git in every
//! worktree, so the answer is cached for a few seconds.

use crate::store::LockRecover;
use axelstra_core::git;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const TTL: Duration = Duration::from_secs(10);

static CACHE: Mutex<Option<(Instant, Value)>> = Mutex::new(None);

/// Key of `dispatch.json`, which the app writes lowercase on every platform.
fn norm(p: &str) -> String {
    axelstra_core::paths::norm(p).to_lowercase()
}

/// Files a worktree changed: committed since the default branch plus uncommitted.
fn changed_files(wt: &Path, base: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    if let Ok(o) = git::run(wt, &["diff", "--name-only", &format!("{base}...HEAD")]) {
        if o.ok {
            out.extend(o.stdout.lines().map(|l| l.trim().replace('\\', "/")).filter(|l| !l.is_empty()));
        }
    }
    // uncommitted: tracked edits (staged or not, renames included) and new files. `git::run` trims the output,
    // which would eat the leading space of the first `status --porcelain` line, so the two listings are asked for apart
    for args in [&["diff", "--name-only", "HEAD"][..], &["ls-files", "--others", "--exclude-standard"][..]] {
        if let Ok(o) = git::run(wt, args) {
            if o.ok {
                out.extend(o.stdout.lines().map(|l| l.trim().replace('\\', "/")).filter(|l| !l.is_empty()));
            }
        }
    }
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

fn compute(store: &Path, dispatch_json: &Path) -> Value {
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
    let mut m = Map::new();
    for (path, others) in invert(&files) {
        m.insert(path, json!(others));
    }
    Value::Object(m)
}

/// `{worktree path: [other worktree paths touching the same files]}`; worktrees without overlaps are absent.
pub fn overlaps(store: &Path, dispatch_json: &Path) -> Value {
    if let Some((at, v)) = CACHE.lock_recover().as_ref() {
        if at.elapsed() < TTL {
            return v.clone();
        }
    }
    let v = compute(store, dispatch_json);
    *CACHE.lock_recover() = Some((Instant::now(), v.clone()));
    v
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
}
