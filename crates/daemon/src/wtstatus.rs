//! One-line git state per worktree (uncommitted files, commits ahead/behind the upstream) for the sidebar badges.
use axelstra_core::git;
use serde::Serialize;
use std::path::Path;

#[derive(Serialize, Debug, PartialEq, Default, Clone)]
pub struct Status {
    pub path: String,
    /// files with uncommitted changes (modified, staged, untracked)
    pub dirty: u32,
    pub ahead: u32,
    pub behind: u32,
    /// true when the branch has an upstream to compare with
    pub upstream: bool,
}

/// Parses `git status --porcelain=v2 --branch` output.
pub fn parse(path: &str, out: &str) -> Status {
    let mut s = Status { path: path.to_string(), ..Default::default() };
    for line in out.lines() {
        if let Some(ab) = line.strip_prefix("# branch.ab ") {
            s.upstream = true;
            for part in ab.split_whitespace() {
                if let Some(n) = part.strip_prefix('+') {
                    s.ahead = n.parse().unwrap_or(0);
                } else if let Some(n) = part.strip_prefix('-') {
                    s.behind = n.parse().unwrap_or(0);
                }
            }
        } else if !line.starts_with('#') && !line.trim().is_empty() {
            s.dirty += 1;
        }
    }
    s
}

/// `status` for the UI's polling: one `git status` per worktree every 4 s at most, however many windows or polls ask.
/// Decisions that delete things (janitor, flow cleanup) call `status` and always see the current state.
pub fn status_cached(worktree: &str) -> Status {
    use std::{
        collections::HashMap,
        sync::Mutex,
        time::{Duration, Instant},
    };
    static CACHE: Mutex<Option<HashMap<String, (Instant, Status)>>> = Mutex::new(None);
    let key = axelstra_core::paths::norm(worktree);
    if let Ok(mut g) = CACHE.lock() {
        let map = g.get_or_insert_with(HashMap::new);
        if let Some((at, st)) = map.get(&key) {
            if at.elapsed() < Duration::from_secs(4) {
                return st.clone();
            }
        }
    }
    let st = status(worktree);
    if let Ok(mut g) = CACHE.lock() {
        let map = g.get_or_insert_with(HashMap::new);
        if map.len() > 256 {
            map.clear();
        }
        map.insert(key, (Instant::now(), st.clone()));
    }
    st
}

/// `status` for the readers the app polls (overview, `worktree.status`): kept while the checkout's HEAD, index, file
/// watch and upstream stay put (`memo::worktree_keep` plus `gitstamp::upstream` for ahead/behind; the age floor only
/// for a checkout the daemon could not watch), so a commit in another worktree does not run it again. Decisions that
/// delete things call `status`.
pub fn status_kept(worktree: &str) -> Status {
    use axelstra_core::gitstamp;
    static KEPT: gitstamp::Stamped<Status> = gitstamp::Stamped::new();
    let k = crate::memo::worktree_keep(worktree);
    let stamp = (|| Some(gitstamp::combine(&[k.stamp?, gitstamp::upstream(std::path::Path::new(worktree))?])))();
    KEPT.get(&axelstra_core::paths::norm(worktree), stamp, k.max_age, || status(worktree))
}

pub fn status(worktree: &str) -> Status {
    match git::run(Path::new(worktree), &["status", "--porcelain=v2", "--branch"]) {
        Ok(o) if o.ok => parse(worktree, &o.stdout),
        _ => Status { path: worktree.to_string(), ..Default::default() },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_dirty_files_and_divergence() {
        let out = "# branch.oid abc\n# branch.head main\n# branch.upstream origin/main\n# branch.ab +2 -1\n1 .M N... 100644 100644 100644 a b src/x.rs\n? new.txt\n";
        let s = parse("p", out);
        assert_eq!((s.dirty, s.ahead, s.behind, s.upstream), (2, 2, 1, true));
    }

    #[test]
    fn no_upstream_means_no_divergence() {
        let s = parse("p", "# branch.oid abc\n# branch.head feat\n");
        assert_eq!((s.dirty, s.ahead, s.behind, s.upstream), (0, 0, 0, false));
    }

    #[test]
    fn a_kept_status_runs_git_again_only_in_the_worktree_that_changed() {
        use crate::memo::testrepo::{cleanup, edit_and_wait, git, git_spawns, with_worktrees};
        let (root, wts) = with_worktrees("status-spawns", 5);
        let paths: Vec<String> = wts.iter().map(|w| w.to_string_lossy().to_string()).collect();
        let all = || paths.iter().for_each(|w| drop(status_kept(w)));
        all();
        assert_eq!(git_spawns(all), 0, "nothing changed");
        edit_and_wait(&wts[1], "own1.txt");
        edit_and_wait(&wts[1], "own1.txt");
        let per: Vec<u64> = paths.iter().map(|w| git_spawns(|| drop(status_kept(w)))).collect();
        assert_eq!(per, vec![0, 1, 0, 0, 0], "only the edited worktree");
        assert_eq!(status_kept(&paths[1]).dirty, 1);
        // a commit in another worktree moves the repo's refs but not this one's HEAD, index or files
        git(&wts[3], &["commit", "-q", "--allow-empty", "-m", "x"]);
        assert_eq!(git_spawns(all), 1, "only the worktree that committed");
        // the upstream of one worktree moved: its ahead/behind is read again, nobody else's
        git(&wts[4], &["branch", "-q", "--set-upstream-to", "main"]);
        all();
        let before = status_kept(&paths[4]).behind;
        git(&root.join("repo"), &["commit", "-q", "--allow-empty", "-m", "m"]);
        assert_eq!(git_spawns(all), 1);
        assert_eq!(status_kept(&paths[4]).behind, before + 1);
        cleanup(&root, &wts);
    }
}
