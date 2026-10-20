use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread::sleep;
use std::time::Duration;

/// `canonicalize` on Windows yields `\\?\` verbatim paths that git rejects.
pub fn canon(p: &Path) -> Result<PathBuf> {
    let c = p.canonicalize()?;
    let s = c.to_string_lossy();
    Ok(match s.strip_prefix(r"\\?\") {
        Some(rest) if !rest.starts_with("UNC") => PathBuf::from(rest),
        _ => c,
    })
}

pub struct Out {
    pub ok: bool,
    pub stdout: String,
    pub stderr: String,
}

pub fn run(cwd: &Path, args: &[&str]) -> Result<Out> {
    let o = Command::new("git")
        .current_dir(cwd)
        .args(args)
        .output()
        .with_context(|| format!("spawn git {args:?}"))?;
    Ok(Out {
        ok: o.status.success(),
        stdout: String::from_utf8_lossy(&o.stdout).trim().to_string(),
        stderr: String::from_utf8_lossy(&o.stderr).trim().to_string(),
    })
}

pub fn must(cwd: &Path, args: &[&str]) -> Result<String> {
    let o = run(cwd, args)?;
    if !o.ok {
        bail!("git {} failed: {}", args.join(" "), o.stderr);
    }
    Ok(o.stdout)
}

/// Absolute common git dir, shared by every worktree of the repo.
pub fn common_dir(repo: &Path) -> Result<PathBuf> {
    let p = must(repo, &["rev-parse", "--path-format=absolute", "--git-common-dir"])?;
    Ok(PathBuf::from(p))
}

pub fn head(repo: &Path) -> Result<String> {
    must(repo, &["rev-parse", "HEAD"])
}

pub fn current_branch(repo: &Path) -> Result<String> {
    must(repo, &["rev-parse", "--abbrev-ref", "HEAD"])
}

pub fn worktree_add(repo: &Path, path: &Path, branch: &str, base: &str) -> Result<()> {
    must(repo, &["worktree", "add", "-b", branch, &path.to_string_lossy(), base])?;
    Ok(())
}

/// Worktree removal often fails on Windows while a handle is still open,
/// so retry with backoff and finally park the directory in a trash folder.
pub fn worktree_remove(repo: &Path, path: &Path) -> Result<()> {
    let p = path.to_string_lossy().to_string();
    for ms in [100u64, 300, 1000, 3000] {
        if run(repo, &["worktree", "remove", "--force", &p])?.ok {
            return Ok(());
        }
        sleep(Duration::from_millis(ms));
    }
    if path.exists() {
        let trash = path.with_file_name(format!(
            ".axelstra-trash-{}",
            path.file_name().map(|s| s.to_string_lossy()).unwrap_or_default()
        ));
        std::fs::rename(path, &trash).with_context(|| format!("park {}", path.display()))?;
    }
    must(repo, &["worktree", "prune"])?;
    Ok(())
}

pub fn branch_delete(repo: &Path, branch: &str) -> Result<()> {
    let _ = run(repo, &["branch", "-D", branch])?;
    Ok(())
}

pub fn has_changes(wt: &Path) -> Result<bool> {
    Ok(!must(wt, &["status", "--porcelain"])?.is_empty())
}

pub fn commit_all(wt: &Path, msg: &str) -> Result<bool> {
    if !has_changes(wt)? {
        return Ok(false);
    }
    must(wt, &["add", "-A"])?;
    must(
        wt,
        &["-c", "user.name=axelstra", "-c", "user.email=axelstra@localhost", "commit", "--no-verify", "-m", msg],
    )?;
    Ok(true)
}

pub enum Merge {
    Clean,
    Conflict,
}

/// Merges `branch` into whatever is checked out in `wt`; aborts on conflict.
pub fn merge(wt: &Path, branch: &str, msg: &str) -> Result<Merge> {
    let o = run(
        wt,
        &[
            "-c", "user.name=axelstra", "-c", "user.email=axelstra@localhost",
            "merge", "--no-ff", "--no-verify", "-m", msg, branch,
        ],
    )?;
    if o.ok {
        return Ok(Merge::Clean);
    }
    let _ = run(wt, &["merge", "--abort"]);
    Ok(Merge::Conflict)
}

pub fn reset_hard(wt: &Path, sha: &str) -> Result<()> {
    must(wt, &["reset", "--hard", sha])?;
    Ok(())
}

pub fn merge_in_progress(wt: &Path) -> bool {
    run(wt, &["rev-parse", "-q", "--verify", "MERGE_HEAD"]).map(|o| o.ok).unwrap_or(false)
}
