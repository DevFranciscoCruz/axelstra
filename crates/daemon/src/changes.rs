//! What changed in a worktree, and the diff of one file.
//! Everything shells out to git so the numbers match what `git diff` shows.

use anyhow::{bail, Context, Result};
use axelstra_core::git;
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Component, Path};

const MAX_DIFF_BYTES: usize = 2 << 20;
const MAX_UNTRACKED_BYTES: u64 = 2 << 20;

#[derive(Debug, Serialize, Clone)]
pub struct FileChange {
    pub path: String,
    /// M modified, A added (incl. untracked), D deleted, U unmerged
    pub status: char,
    pub untracked: bool,
    pub staged: bool,
    pub added: u32,
    pub removed: u32,
    pub binary: bool,
}

#[derive(Debug, Serialize)]
pub struct Changes {
    /// commit the diff is relative to (HEAD, or the merge-base with `base`)
    pub against: String,
    pub files: Vec<FileChange>,
}

/// Commit to diff against: HEAD, or the merge-base of HEAD and `base` for a branch-wide view.
fn reference(wt: &Path, base: Option<&str>) -> Result<String> {
    match base {
        None => git::must(wt, &["rev-parse", "HEAD"]),
        Some(b) => git::must(wt, &["merge-base", "HEAD", b])
            .with_context(|| format!("no common history with '{b}'")),
    }
}

pub fn changes(wt: &Path, base: Option<&str>) -> Result<Changes> {
    let against = reference(wt, base)?;

    let o = git::run(wt, &["-c", "status.renames=false", "status", "--porcelain=v2", "-z", "--untracked-files=all"])?;
    if !o.ok {
        bail!("git status: {}", o.stderr);
    }
    let mut files: Vec<FileChange> = Vec::new();
    for rec in o.stdout.split('\0').filter(|r| !r.is_empty()) {
        let kind = rec.as_bytes()[0];
        match kind {
            b'1' => {
                // 1 XY sub mH mI mW hH hI path
                let f: Vec<&str> = rec.splitn(9, ' ').collect();
                let (xy, path) = (f[1].as_bytes(), f[8]);
                let code = if xy[0] != b'.' { xy[0] } else { xy[1] };
                files.push(FileChange {
                    path: path.to_string(),
                    status: code as char,
                    untracked: false,
                    staged: xy[0] != b'.',
                    added: 0,
                    removed: 0,
                    binary: false,
                });
            }
            b'u' => {
                let path = rec.splitn(11, ' ').last().unwrap_or_default();
                files.push(FileChange { path: path.into(), status: 'U', untracked: false, staged: false, added: 0, removed: 0, binary: false });
            }
            b'?' => {
                let path = &rec[2..];
                let (added, binary) = count_lines(&wt.join(path));
                files.push(FileChange { path: path.into(), status: 'A', untracked: true, staged: false, added, removed: 0, binary });
            }
            _ => {}
        }
    }

    // line counts for tracked files, relative to `against` (working tree included)
    let n = git::run(wt, &["diff", &against, "--numstat", "-z", "--no-renames"])?;
    let mut counts: HashMap<String, (u32, u32, bool)> = HashMap::new();
    for rec in n.stdout.split('\0').filter(|r| !r.is_empty()) {
        let mut it = rec.splitn(3, '\t');
        let (a, r, p) = (it.next().unwrap_or("-"), it.next().unwrap_or("-"), it.next().unwrap_or(""));
        counts.insert(p.to_string(), (a.parse().unwrap_or(0), r.parse().unwrap_or(0), a == "-"));
    }
    for f in &mut files {
        if let Some((a, r, bin)) = counts.remove(&f.path) {
            f.added = a;
            f.removed = r;
            f.binary = bin;
        }
    }
    // branch view: files committed since the merge-base but clean in the working tree
    for (p, (a, r, bin)) in counts {
        files.push(FileChange { path: p, status: 'M', untracked: false, staged: false, added: a, removed: r, binary: bin });
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(Changes { against, files })
}

/// (line count, looks binary)
fn count_lines(p: &Path) -> (u32, bool) {
    let Ok(md) = std::fs::metadata(p) else { return (0, false) };
    if md.len() > MAX_UNTRACKED_BYTES {
        return (0, false);
    }
    match std::fs::read(p) {
        Ok(b) if b.contains(&0) => (0, true),
        Ok(b) => (b.iter().filter(|&&c| c == b'\n').count() as u32 + u32::from(!b.is_empty() && !b.ends_with(b"\n")), false),
        Err(_) => (0, false),
    }
}

/// Only plain relative paths inside the worktree are accepted.
fn safe_rel(path: &str) -> Result<()> {
    let p = Path::new(path);
    if path.is_empty() || p.is_absolute() || p.components().any(|c| !matches!(c, Component::Normal(_))) {
        bail!("invalid path");
    }
    Ok(())
}

/// Unified diff of one file. Untracked files are shown as entirely added.
pub fn diff(wt: &Path, path: &str, base: Option<&str>) -> Result<String> {
    safe_rel(path)?;
    let against = reference(wt, base)?;
    let tracked = git::run(wt, &["ls-files", "--error-unmatch", "--", path])?.ok;
    let mut out = if tracked {
        let o = git::run(wt, &["diff", &against, "--no-color", "--no-ext-diff", "--no-renames", "-U3", "--", path])?;
        if !o.ok {
            bail!("git diff: {}", o.stderr);
        }
        o.stdout
    } else {
        let full = wt.join(path);
        if !full.is_file() {
            // deleted in the working tree and unknown to the index: nothing to show
            String::new()
        } else {
            let bytes = std::fs::read(&full)?;
            if bytes.contains(&0) {
                format!("Binary file {path} (new)\n")
            } else {
                let text = String::from_utf8_lossy(&bytes);
                let lines: Vec<&str> = text.lines().collect();
                let mut s = format!("--- /dev/null\n+++ b/{path}\n@@ -0,0 +1,{} @@\n", lines.len());
                for l in lines {
                    s.push('+');
                    s.push_str(l);
                    s.push('\n');
                }
                s
            }
        }
    };
    if out.len() > MAX_DIFF_BYTES {
        out.truncate(MAX_DIFF_BYTES);
        out.push_str("\n… diff truncated (too large)\n");
    }
    Ok(out)
}
