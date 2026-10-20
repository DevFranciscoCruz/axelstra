//! Commit / push / pull request for a worktree. These use the user's own git
//! identity and hooks (unlike orchestrator commits). Nothing here runs unless
//! the UI user asks for it explicitly.

use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::path::Path;
use std::process::Command;

fn exec(cwd: &Path, prog: &str, args: &[&str]) -> Result<(bool, String)> {
    let o = Command::new(prog)
        .current_dir(cwd)
        // never block waiting for a credential prompt nobody can see
        .env("GIT_TERMINAL_PROMPT", "0")
        .args(args)
        .output()
        .with_context(|| format!("could not start {prog}"))?;
    let mut text = String::from_utf8_lossy(&o.stdout).trim().to_string();
    let err = String::from_utf8_lossy(&o.stderr).trim().to_string();
    if !err.is_empty() {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(&err);
    }
    Ok((o.status.success(), text))
}

fn branch(wt: &Path) -> Result<String> {
    let (ok, b) = exec(wt, "git", &["symbolic-ref", "--short", "-q", "HEAD"])?;
    if !ok || b.is_empty() {
        bail!("detached HEAD: switch to a branch first");
    }
    Ok(b)
}

#[derive(Serialize)]
pub struct Committed {
    pub sha: String,
}

/// Stage everything and commit with the user's identity.
pub fn commit(wt: &Path, message: &str) -> Result<Committed> {
    let message = message.trim();
    if message.is_empty() {
        bail!("commit message is empty");
    }
    let (ok, out) = exec(wt, "git", &["add", "-A"])?;
    if !ok {
        bail!("git add failed: {out}");
    }
    let (staged_none, _) = exec(wt, "git", &["diff", "--cached", "--quiet"])?;
    if staged_none {
        bail!("nothing to commit");
    }
    let (ok, out) = exec(wt, "git", &["commit", "-m", message])?;
    if !ok {
        bail!("git commit failed: {out}");
    }
    let (_, sha) = exec(wt, "git", &["rev-parse", "--short", "HEAD"])?;
    Ok(Committed { sha })
}

/// Push the current branch to `origin` and set its upstream.
pub fn push(wt: &Path) -> Result<String> {
    let b = branch(wt)?;
    let (ok, out) = exec(wt, "git", &["push", "-u", "origin", &b])?;
    if !ok {
        bail!("git push failed: {out}");
    }
    Ok(out)
}

/// Open a pull request with the GitHub CLI; returns the PR URL.
pub fn pull_request(wt: &Path, title: &str, body: &str, base: Option<&str>) -> Result<String> {
    let title = title.trim();
    if title.is_empty() {
        bail!("pull request title is empty");
    }
    let b = branch(wt)?;
    let mut args = vec!["pr", "create", "--head", b.as_str(), "--title", title, "--body", body];
    if let Some(base) = base.filter(|s| !s.is_empty()) {
        args.extend(["--base", base]);
    }
    let (ok, out) = exec(wt, "gh", &args).map_err(|e| anyhow::anyhow!("{e}. Install the GitHub CLI (gh) and run `gh auth login`."))?;
    if !ok {
        bail!("gh pr create failed: {out}");
    }
    // gh prints the URL as the last line
    Ok(out.lines().last().unwrap_or_default().trim().to_string())
}
