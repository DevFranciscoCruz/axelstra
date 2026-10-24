//! What changed in a worktree, and the diff of one file.
//! Everything shells out to git so the numbers match what `git diff` shows.

use anyhow::Result;
use axelstra_core::{coded, git};
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

/// A failed git command as an error: git's own words in the English text, a message code for the usual causes.
fn git_err(cmd: &str, stderr: &str) -> anyhow::Error {
    let code = git::classify_failure(stderr).unwrap_or("git.failed");
    axelstra_core::msg::Coded::new(code, format!("git {cmd}: {stderr}"), &[("cmd", cmd.to_string()), ("detail", stderr.to_string())])
        .into_error()
}

/// The commit a caller-supplied ref names. A ref starting with a dash would act as a git option, and anything
/// that is not an existing commit is refused before it reaches another command.
fn resolve_base(wt: &Path, b: &str) -> Result<String> {
    if b.is_empty() || b.starts_with('-') || b.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err(coded!("change.invalid_base", "invalid base ref '{base}'", base = b));
    }
    git::must(wt, &["rev-parse", "--verify", "--quiet", "--end-of-options", &format!("{b}^{{commit}}")])
        .map_err(|e| coded!("change.unknown_base", "unknown base ref '{base}': {detail}", base = b, detail = format!("{e:#}")))
}

/// Commit to diff against: HEAD, or the merge-base of HEAD and `base` for a branch-wide view.
fn reference(wt: &Path, base: Option<&str>) -> Result<String> {
    match base {
        None => git::must(wt, &["rev-parse", "HEAD"]),
        Some(b) => {
            let commit = resolve_base(wt, b)?;
            git::must(wt, &["merge-base", "HEAD", &commit]).map_err(|e| {
                coded!("change.no_common_history", "no common history with '{base}': {detail}", base = b, detail = format!("{e:#}"))
            })
        }
    }
}

pub fn changes(wt: &Path, base: Option<&str>) -> Result<Changes> {
    let against = reference(wt, base)?;

    let o = git::run(wt, &["-c", "status.renames=false", "status", "--porcelain=v2", "-z", "--untracked-files=all"])?;
    if !o.ok {
        return Err(git_err("status", &o.stderr));
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
                files.push(FileChange {
                    path: path.into(),
                    status: 'U',
                    untracked: false,
                    staged: false,
                    added: 0,
                    removed: 0,
                    binary: false,
                });
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
        return Err(coded!("fs.invalid_path", "invalid path"));
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
            return Err(git_err("diff", &o.stderr));
        }
        o.stdout
    } else {
        let full = wt.join(path);
        // symlink_metadata: a link is never followed (it could point outside the worktree)
        let md = std::fs::symlink_metadata(&full).ok().filter(|m| m.file_type().is_file());
        if md.is_none() {
            // deleted in the working tree, unknown to the index, or a link: nothing to show
            String::new()
        } else {
            let mut bytes = Vec::new();
            std::io::Read::read_to_end(&mut std::io::Read::take(std::fs::File::open(&full)?, MAX_UNTRACKED_BYTES + 1), &mut bytes)?;
            if bytes.len() as u64 > MAX_UNTRACKED_BYTES {
                format!("File {path} (new) is larger than {} MB\n", MAX_UNTRACKED_BYTES >> 20)
            } else if bytes.contains(&0) {
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

#[derive(Debug, Serialize)]
pub struct Commit {
    pub sha: String,
    pub short: String,
    pub author: String,
    pub date: String,
    pub subject: String,
    pub files: u32,
    pub added: u32,
    pub removed: u32,
}

/// Commits on HEAD that `base` does not have (newest first). Without a base: the last 50.
pub fn log(wt: &Path, base: Option<&str>) -> Result<Vec<Commit>> {
    let range = base.map(|b| resolve_base(wt, b).map(|c| format!("{c}..HEAD"))).transpose()?;
    let mut args = vec!["log", "--no-color", "--no-renames", "-n", "50", "--numstat", "--format=%x1e%H%x1f%h%x1f%an%x1f%aI%x1f%s"];
    if let Some(r) = &range {
        args.push(r);
    }
    args.push("--");
    let o = git::run(wt, &args)?;
    if !o.ok {
        return Err(git_err("log", &o.stderr));
    }
    let mut out = Vec::new();
    for rec in o.stdout.split('\u{1e}').filter(|r| !r.trim().is_empty()) {
        let mut lines = rec.lines();
        let head: Vec<&str> = lines.next().unwrap_or_default().split('\u{1f}').collect();
        if head.len() < 5 {
            continue;
        }
        let (mut files, mut added, mut removed) = (0, 0, 0);
        for l in lines {
            let f: Vec<&str> = l.split('\t').collect();
            if f.len() == 3 {
                files += 1;
                // binary files show "-"
                added += f[0].parse::<u32>().unwrap_or(0);
                removed += f[1].parse::<u32>().unwrap_or(0);
            }
        }
        out.push(Commit {
            sha: head[0].into(),
            short: head[1].into(),
            author: head[2].into(),
            date: head[3].into(),
            subject: head[4].into(),
            files,
            added,
            removed,
        });
    }
    Ok(out)
}

/// Full patch of one commit. `sha` must be a plain hex id so it can never act as an option.
pub fn show(wt: &Path, sha: &str) -> Result<String> {
    if !(7..=40).contains(&sha.len()) || !sha.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(coded!("change.invalid_commit", "invalid commit id"));
    }
    let o = git::run(wt, &["show", "--no-color", "--no-ext-diff", "--no-renames", "-U3", "--format=", sha])?;
    if !o.ok {
        return Err(git_err("show", &o.stderr));
    }
    let mut s = o.stdout;
    if s.len() > MAX_DIFF_BYTES {
        let mut cut = MAX_DIFF_BYTES;
        while !s.is_char_boundary(cut) {
            cut -= 1;
        }
        s.truncate(cut);
        s.push_str("\n… diff truncated\n");
    }
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_carry_message_codes() {
        let here = std::env::temp_dir();
        let c = |e: anyhow::Error| axelstra_core::msg::find(&e).map(|c| c.code.clone()).unwrap_or_default();
        assert_eq!(c(resolve_base(&here, "-n1").unwrap_err()), "change.invalid_base");
        assert_eq!(c(show(&here, "zz").unwrap_err()), "change.invalid_commit");
        assert_eq!(c(diff(&here, "../x", None).unwrap_err()), "fs.invalid_path");
        // git's own failure keeps its classified code
        let e = git_err("status", "fatal: not a git repository");
        assert_eq!(c(e), "git.not_repo");
        assert_eq!(c(git_err("status", "weird")), "git.failed");
    }

    #[test]
    fn a_base_that_looks_like_an_option_is_refused() {
        let here = std::env::temp_dir();
        for bad in ["--output=/tmp/x", "-n1", "", "a b", "main\n--all"] {
            let e = resolve_base(&here, bad).unwrap_err().to_string();
            assert!(e.contains("invalid base ref"), "{bad:?}: {e}");
            assert!(log(&here, Some(bad)).is_err());
            assert!(changes(&here, Some(bad)).is_err());
        }
    }

    fn repo(tag: &str) -> Option<std::path::PathBuf> {
        let d = std::env::temp_dir().join(format!("axl-changes-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).ok()?;
        for args in [vec!["init", "-q"], vec!["-c", "user.name=t", "-c", "user.email=t@t", "commit", "--allow-empty", "-q", "-m", "i"]] {
            if !git::run(&d, &args).ok()?.ok {
                return None;
            }
        }
        Some(d)
    }

    #[test]
    fn untracked_diff_is_bounded_and_never_follows_a_link() {
        let Some(d) = repo("untracked") else { return };
        std::fs::write(d.join("small.txt"), "a\nb\n").unwrap();
        let s = diff(&d, "small.txt", None).unwrap();
        assert!(s.contains("+a\n+b\n"), "{s}");
        std::fs::write(d.join("big.txt"), vec![b'x'; (MAX_UNTRACKED_BYTES + 10) as usize]).unwrap();
        let s = diff(&d, "big.txt", None).unwrap();
        assert!(s.contains("larger than") && s.len() < 200, "{}", s.len());
        // a link to a file outside the worktree shows nothing
        let outside = std::env::temp_dir().join(format!("axl-changes-out-{}.txt", std::process::id()));
        std::fs::write(&outside, "secret\n").unwrap();
        #[cfg(unix)]
        let linked = std::os::unix::fs::symlink(&outside, d.join("ln")).is_ok();
        #[cfg(windows)]
        let linked = std::os::windows::fs::symlink_file(&outside, d.join("ln")).is_ok();
        if linked {
            assert_eq!(diff(&d, "ln", None).unwrap(), "");
        } else {
            eprintln!("skipped link half: cannot create symlinks here");
        }
        let _ = std::fs::remove_file(&outside);
        let _ = std::fs::remove_dir_all(&d);
    }
}
