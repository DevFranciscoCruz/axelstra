//! Commit / push / pull request for a worktree. These use the user's own git
//! identity and hooks (unlike orchestrator commits). Nothing here runs unless
//! the UI user asks for it explicitly.

use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::path::Path;
use std::process::Command;

fn exec(cwd: &Path, prog: &str, args: &[&str]) -> Result<(bool, String)> {
    let o = axelstra_core::git::quiet(&mut Command::new(prog))
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

/// Most diff text the agent is shown; the file list still covers everything.
const SUGGEST_DIFF_CAP: usize = 24_000;
const SUGGEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(90);

fn cap_chars(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

fn suggest_prompt(subjects: &str, status: &str, diff: &str) -> String {
    let shown = cap_chars(diff, SUGGEST_DIFF_CAP);
    let cut = if shown.len() < diff.len() { "\n[diff truncated]" } else { "" };
    format!(
        "You write git commit messages. Reply with the commit message only: no quotes, no code fences, no explanation.\n\
         Rules:\n\
         - Subject line in the imperative mood, at most 72 characters, no trailing period.\n\
         - Follow the style of the recent subjects below: reuse their prefix convention (e.g. Conventional Commits) and their language.\n\
         - Say what the change does and why, not how. Do not list files.\n\
         - Add a body, wrapped at 72 columns after a blank line, only when the reason is not obvious from the subject.\n\
         - One logical change per message: if the diff mixes concerns, name the main one.\n\n\
         Recent subjects:\n{subjects}\n\nChanged files:\n{status}\n\nDiff:\n{shown}{cut}\n"
    )
}

/// The agent may wrap its answer in fences or quotes; keep only the message.
fn clean_message(raw: &str) -> String {
    let kept: Vec<&str> = raw.lines().filter(|l| !l.trim_start().starts_with("```")).collect();
    let text = kept.join("\n");
    let text = text.trim();
    let unquoted = text.strip_prefix('"').and_then(|t| t.strip_suffix('"')).unwrap_or(text);
    unquoted.trim().to_string()
}

fn run_agent(prompt: &str) -> Result<String> {
    use std::io::{Read, Write};
    use std::process::Stdio;
    // `cmd /C` so a `claude.cmd` shim is found on Windows too; the prompt goes through stdin, never the command line
    let mut cmd = if cfg!(windows) {
        let mut c = Command::new("cmd");
        c.args(["/C", "claude", "-p"]);
        c
    } else {
        let mut c = Command::new("claude");
        c.arg("-p");
        c
    };
    axelstra_core::git::quiet(&mut cmd);
    // outside the repo, so the agent works from the prompt alone and does not wander through the project
    let mut child = cmd
        .current_dir(std::env::temp_dir())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| anyhow::anyhow!("the `claude` CLI was not found on PATH"))?;
    let mut stdin = child.stdin.take().context("no stdin")?;
    let text = prompt.to_string();
    std::thread::spawn(move || {
        let _ = stdin.write_all(text.as_bytes());
    });
    let (mut out, mut err) = (child.stdout.take().context("no stdout")?, child.stderr.take().context("no stderr")?);
    let out_t = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = out.read_to_string(&mut s);
        s
    });
    let err_t = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = err.read_to_string(&mut s);
        s
    });
    let deadline = std::time::Instant::now() + SUGGEST_TIMEOUT;
    let status = loop {
        if let Some(s) = child.try_wait()? {
            break s;
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            bail!("the agent took longer than {}s", SUGGEST_TIMEOUT.as_secs());
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    };
    let (out, err) = (out_t.join().unwrap_or_default(), err_t.join().unwrap_or_default());
    if !status.success() {
        bail!("claude failed: {}", cap_chars(err.trim(), 300));
    }
    Ok(out)
}

/// Asks the user's `claude` CLI for a commit message that fits the pending changes and the repo's recent style.
/// Read-only: nothing is staged or committed.
pub fn suggest_message(wt: &Path) -> Result<String> {
    let (_, status) = exec(wt, "git", &["status", "--short"])?;
    if status.trim().is_empty() {
        bail!("nothing to commit");
    }
    let (ok, diff) = exec(wt, "git", &["diff", "HEAD"])?;
    // no commits yet: fall back to what is staged
    let diff = if ok { diff } else { exec(wt, "git", &["diff", "--cached"])?.1 };
    let (_, subjects) = exec(wt, "git", &["log", "-n", "10", "--format=%s"])?;
    let msg = clean_message(&run_agent(&suggest_prompt(subjects.trim(), status.trim(), &diff))?);
    if msg.is_empty() {
        bail!("the agent returned no message");
    }
    Ok(msg)
}

#[cfg(test)]
mod suggest_tests {
    use super::*;

    #[test]
    fn answers_wrapped_in_fences_or_quotes_are_unwrapped() {
        assert_eq!(clean_message("```\nfeat: add x\n```\n"), "feat: add x");
        assert_eq!(clean_message("\"fix: y\""), "fix: y");
        assert_eq!(clean_message("feat: z\n\nbody line"), "feat: z\n\nbody line");
    }

    #[test]
    fn the_prompt_caps_the_diff_on_a_char_boundary_and_says_so() {
        let diff = "é".repeat(SUGGEST_DIFF_CAP);
        let p = suggest_prompt("feat: a", "M a.rs", &diff);
        assert!(p.contains("[diff truncated]"));
        assert!(p.contains("feat: a") && p.contains("M a.rs"));
        assert!(!suggest_prompt("", "", "small").contains("[diff truncated]"));
    }

    /// Needs the `claude` CLI and a login, so it is not part of the normal run: `cargo test -p axelstra-daemon -- --ignored suggest`.
    #[test]
    #[ignore]
    fn suggests_a_message_for_a_real_repo() {
        let dir = std::env::temp_dir().join(format!("axl-sug-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let g = |args: &[&str]| assert!(exec(&dir, "git", args).unwrap().0, "git {args:?}");
        g(&["init", "-q", "-b", "main"]);
        std::fs::write(dir.join("a.txt"), "one
").unwrap();
        g(&["add", "-A"]);
        g(&["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "-m", "feat: add a.txt"]);
        std::fs::write(dir.join("a.txt"), "one
two
").unwrap();
        let m = suggest_message(&dir).unwrap();
        eprintln!("suggested: {m}");
        assert!(!m.trim().is_empty() && !m.contains("```"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
