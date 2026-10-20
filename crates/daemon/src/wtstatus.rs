//! One-line git state per worktree (uncommitted files, commits ahead/behind the upstream) for the sidebar badges.
use axelstra_core::git;
use serde::Serialize;
use std::path::Path;

#[derive(Serialize, Debug, PartialEq, Default)]
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
}
