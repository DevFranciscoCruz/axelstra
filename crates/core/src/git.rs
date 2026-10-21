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

/// The daemon has no console, so a console child (git, gh, cmd) would flash a new window
/// every time it starts. Call this on every Command the daemon spawns itself.
pub fn quiet(cmd: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    cmd
}

pub fn run(cwd: &Path, args: &[&str]) -> Result<Out> {
    let o = quiet(&mut Command::new("git"))
        .current_dir(cwd)
        // read-only queries must not rewrite .git/index: the watcher would see it and loop
        .env("GIT_OPTIONAL_LOCKS", "0")
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

/// `git worktree list` is asked many times a second by the UI and the daemon (every folder check starts a process):
/// the answer is reused for a moment and dropped whenever a worktree is added or removed here.
static WT_CACHE: std::sync::Mutex<Option<std::collections::HashMap<String, (std::time::Instant, Vec<Worktree>)>>> = std::sync::Mutex::new(None);

pub fn forget_worktree_lists() {
    *WT_CACHE.lock().unwrap() = None;
}

pub fn worktree_add(repo: &Path, path: &Path, branch: &str, base: &str) -> Result<()> {
    forget_worktree_lists();
    must(repo, &["worktree", "add", "-b", branch, &path.to_string_lossy(), base])?;
    Ok(())
}

/// Worktree removal often fails on Windows while a handle is still open,
/// so retry with backoff and finally park the directory in a trash folder.
pub fn worktree_remove(repo: &Path, path: &Path) -> Result<()> {
    forget_worktree_lists();
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
        // the parked folder (node_modules and all) is deleted in the background once whatever held it lets go
        if let Some(dir) = trash.parent().map(Path::to_path_buf) {
            std::thread::spawn(move || {
                for _ in 0..6 {
                    sleep(Duration::from_secs(10));
                    purge_trash(&dir);
                }
            });
        }
    }
    must(repo, &["worktree", "prune"])?;
    Ok(())
}

/// Deletes the `.axelstra-trash-*` folders left in `dir` by removals that had to park a worktree.
pub fn purge_trash(dir: &Path) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        if e.file_name().to_string_lossy().starts_with(".axelstra-trash-") {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
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

#[derive(Debug, Clone, serde::Serialize)]
pub struct Worktree {
    pub path: String,
    pub branch: Option<String>,
    pub head: String,
    pub is_main: bool,
}

/// Top-level directory of the repo containing `path`, or an error if it is not one.
pub fn repo_root(path: &Path) -> Result<PathBuf> {
    let top = must(path, &["rev-parse", "--show-toplevel"])?;
    canon(Path::new(&top))
}

pub fn worktree_list(repo: &Path) -> Result<Vec<Worktree>> {
    let key = repo.to_string_lossy().to_string();
    if let Some((at, v)) = WT_CACHE.lock().unwrap().as_ref().and_then(|m| m.get(&key)) {
        if at.elapsed() < Duration::from_secs(3) {
            return Ok(v.clone());
        }
    }
    let v = worktree_list_uncached(repo)?;
    WT_CACHE.lock().unwrap().get_or_insert_with(Default::default).insert(key, (std::time::Instant::now(), v.clone()));
    Ok(v)
}

fn worktree_list_uncached(repo: &Path) -> Result<Vec<Worktree>> {
    let out = must(repo, &["worktree", "list", "--porcelain"])?;
    let mut v: Vec<Worktree> = Vec::new();
    for block in out.split("\n\n") {
        let mut w = Worktree { path: String::new(), branch: None, head: String::new(), is_main: v.is_empty() };
        for l in block.lines() {
            if let Some(p) = l.strip_prefix("worktree ") {
                w.path = p.replace('/', std::path::MAIN_SEPARATOR_STR);
            } else if let Some(h) = l.strip_prefix("HEAD ") {
                w.head = h.to_string();
            } else if let Some(b) = l.strip_prefix("branch ") {
                w.branch = Some(b.strip_prefix("refs/heads/").unwrap_or(b).to_string());
            }
        }
        if !w.path.is_empty() {
            v.push(w);
        }
    }
    Ok(v)
}

/// `(owner, repo)` when the URL points at github.com. Handles https, ssh and scp-style remotes.
pub fn parse_github(url: &str) -> Option<(String, String)> {
    let u = url.trim();
    let rest = u
        .strip_prefix("git@github.com:")
        .or_else(|| u.strip_prefix("ssh://git@github.com/"))
        .or_else(|| u.strip_prefix("git://github.com/"))
        .or_else(|| {
            // https://[user[:token]@]github.com/owner/repo
            let after = u.strip_prefix("https://").or_else(|| u.strip_prefix("http://"))?;
            let after = after.rsplit_once('@').map(|(_, h)| h).unwrap_or(after);
            after.strip_prefix("github.com/")
        })?;
    let rest = rest.trim_end_matches('/').trim_end_matches(".git");
    let (owner, repo) = rest.split_once('/')?;
    if owner.is_empty() || repo.is_empty() || repo.contains('/') {
        return None;
    }
    Some((owner.to_string(), repo.to_string()))
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct RepoIdentity {
    /// repo name from the origin remote, else the folder name
    pub display_name: String,
    /// "owner/repo" for github remotes
    pub slug: Option<String>,
    /// the owner's GitHub avatar, the same logo shows
    pub icon_url: Option<String>,
    pub remote: Option<String>,
}

/// The branch the repo's main worktree is meant to stay on: origin's default, else `main`, else `master`.
pub fn default_branch(repo: &Path) -> String {
    if let Ok(o) = run(repo, &["symbolic-ref", "--short", "-q", "refs/remotes/origin/HEAD"]) {
        if o.ok {
            if let Some(b) = o.stdout.strip_prefix("origin/") {
                return b.to_string();
            }
        }
    }
    for b in ["main", "master"] {
        if run(repo, &["show-ref", "--verify", "--quiet", &format!("refs/heads/{b}")]).map(|o| o.ok).unwrap_or(false) {
            return b.to_string();
        }
    }
    "main".to_string()
}

pub fn identity(repo: &Path) -> RepoIdentity {
    let folder = repo.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let remote = run(repo, &["config", "--get", "remote.origin.url"]).ok().filter(|o| o.ok && !o.stdout.is_empty()).map(|o| o.stdout);
    // never expose credentials embedded in a remote url
    let shown = remote.as_deref().map(|u| match (u.find("://"), u.rfind('@')) {
        (Some(i), Some(a)) if a > i => format!("{}{}", &u[..i + 3], &u[a + 1..]),
        _ => u.to_string(),
    });
    match remote.as_deref().and_then(parse_github) {
        Some((owner, name)) => RepoIdentity {
            display_name: name.clone(),
            slug: Some(format!("{owner}/{name}")),
            icon_url: Some(format!("https://github.com/{owner}.png?size=64")),
            remote: shown,
        },
        None => {
            let name = shown
                .as_deref()
                .and_then(|u| u.trim_end_matches('/').trim_end_matches(".git").rsplit(['/', ':']).next().map(String::from))
                .filter(|n| !n.is_empty())
                .unwrap_or(folder);
            RepoIdentity { display_name: name, slug: None, icon_url: None, remote: shown }
        }
    }
}

#[cfg(test)]
mod identity_tests {
    use super::parse_github;

    #[test]
    fn parses_github_remotes() {
        let want = Some(("Scorpio-Bots".to_string(), "scorpio-bot-reborn".to_string()));
        for u in [
            "https://github.com/Scorpio-Bots/scorpio-bot-reborn.git",
            "https://github.com/Scorpio-Bots/scorpio-bot-reborn",
            "https://user:token@github.com/Scorpio-Bots/scorpio-bot-reborn.git",
            "git@github.com:Scorpio-Bots/scorpio-bot-reborn.git",
            "ssh://git@github.com/Scorpio-Bots/scorpio-bot-reborn.git",
            "https://github.com/Scorpio-Bots/scorpio-bot-reborn/",
        ] {
            assert_eq!(parse_github(u), want, "{u}");
        }
        assert_eq!(parse_github("https://gitlab.com/a/b.git"), None);
        assert_eq!(parse_github("https://github.com/onlyowner"), None);
        assert_eq!(parse_github("/local/path/repo"), None);
    }
}
