use crate::coded;
use crate::sync::LockRecover;
use anyhow::Result;
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

#[derive(Debug)]
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

/// Default deadline of `run`: generous for a big fetch or checkout, finite so a stuck git cannot block the caller forever.
pub const RUN_TIMEOUT: Duration = Duration::from_secs(120);
/// Most output of one git command that is kept; more is an error rather than a silently cut answer.
const RUN_MAX_BYTES: usize = 64 << 20;

pub fn run(cwd: &Path, args: &[&str]) -> Result<Out> {
    run_timeout(cwd, args, RUN_TIMEOUT)
}

/// `run` with an explicit deadline. git never prompts (`GIT_TERMINAL_PROMPT=0`, `GCM_INTERACTIVE=never`): a missing
/// credential fails the command instead of waiting for a terminal or credential-manager dialog nobody will answer.
/// On timeout the whole git process tree is killed and an error is returned.
pub fn run_timeout(cwd: &Path, args: &[&str], limit: Duration) -> Result<Out> {
    let mut cmd = Command::new("git");
    cmd.current_dir(cwd)
        // read-only queries must not rewrite .git/index: the watcher would see it and loop
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GCM_INTERACTIVE", "never");
    // deep node_modules / target trees pass 260 characters: let git on Windows open them
    if cfg!(windows) {
        cmd.args(["-c", "core.longpaths=true"]);
    }
    cmd.args(args);
    let o = crate::exec::run_capped(&mut cmd, limit, RUN_MAX_BYTES).map_err(|e| {
        coded!("git.spawn", "could not start git {cmd}: {detail}", cmd = args.first().unwrap_or(&""), detail = format!("{e:#}"))
    })?;
    if o.timed_out {
        return Err(coded!("git.timeout", "git {cmd} timed out after {secs}s", cmd = args.first().unwrap_or(&""), secs = limit.as_secs()));
    }
    if o.truncated {
        return Err(coded!(
            "git.too_big",
            "git {cmd} printed more than {mib} MiB",
            cmd = args.first().unwrap_or(&""),
            mib = RUN_MAX_BYTES >> 20
        ));
    }
    Ok(Out { ok: o.success(), stdout: o.stdout_lossy().trim().to_string(), stderr: o.stderr_lossy().trim().to_string() })
}

pub fn must(cwd: &Path, args: &[&str]) -> Result<String> {
    let o = run(cwd, args)?;
    if !o.ok {
        return Err(failure(args, &o.stderr));
    }
    Ok(o.stdout)
}

/// The error for a git command that exited non-zero: `git.failed` unless its stderr is one of the usual causes, which
/// get a code of their own so the UI can say what to do (the English text keeps git's own words).
pub fn failure(args: &[&str], stderr: &str) -> anyhow::Error {
    let (cmd, detail) = (args.join(" "), stderr);
    match classify_failure(stderr) {
        Some("git.not_repo") => coded!("git.not_repo", "git {cmd} failed: {detail}", cmd = cmd, detail = detail),
        Some("git.dirty") => coded!("git.dirty", "git {cmd} failed: {detail}", cmd = cmd, detail = detail),
        Some("git.exists") => coded!("git.exists", "git {cmd} failed: {detail}", cmd = cmd, detail = detail),
        Some("git.remote") => coded!("git.remote", "git {cmd} failed: {detail}", cmd = cmd, detail = detail),
        Some("git.rejected") => coded!("git.rejected", "git {cmd} failed: {detail}", cmd = cmd, detail = detail),
        Some("git.conflicts") => coded!("git.conflicts", "git {cmd} failed: {detail}", cmd = cmd, detail = detail),
        Some("git.nothing_to_commit") => coded!("git.nothing_to_commit", "git {cmd} failed: {detail}", cmd = cmd, detail = detail),
        Some("git.bad_ref") => coded!("git.bad_ref", "git {cmd} failed: {detail}", cmd = cmd, detail = detail),
        _ => coded!("git.failed", "git {cmd} failed: {detail}", cmd = cmd, detail = detail),
    }
}

/// Which usual cause git's stderr names, as a message code.
pub fn classify_failure(stderr: &str) -> Option<&'static str> {
    let m = stderr.to_lowercase();
    let has = |needles: &[&str]| needles.iter().any(|n| m.contains(n));
    if has(&["not a git repository"]) {
        Some("git.not_repo")
    } else if has(&["contains modified or untracked files", "local changes would be overwritten"]) {
        Some("git.dirty")
    } else if has(&["is already checked out", "already exists"]) {
        Some("git.exists")
    } else if has(&["could not read from remote", "authentication failed", "permission denied (publickey)", "could not resolve host"]) {
        Some("git.remote")
    } else if has(&["non-fast-forward", "failed to push", "rejected"]) {
        Some("git.rejected")
    } else if has(&["merge conflict", "automatic merge failed", "conflict ("]) {
        Some("git.conflicts")
    } else if has(&["nothing to commit", "no changes added to commit"]) {
        Some("git.nothing_to_commit")
    } else if has(&["did not match", "unknown revision", "bad revision", "not a valid object name", "invalid reference"]) {
        Some("git.bad_ref")
    } else {
        None
    }
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

/// File in a worktree's own git dir that marks its code as not the user's (a pull request's).
pub const UNTRUSTED_MARK: &str = "axelstra-untrusted";

/// Marks the worktree at `dir` as holding code the user did not write, so no agent is told to trust the folder.
pub fn mark_untrusted(dir: &Path) -> Result<()> {
    let gd = must(dir, &["rev-parse", "--path-format=absolute", "--git-dir"])?;
    std::fs::write(
        Path::new(&gd).join(UNTRUSTED_MARK),
        "pull request
",
    )?;
    Ok(())
}

/// Whether an unattended agent may be told to trust the folder `dir` (`--skip-trust`): only a worktree on an
/// `ax/*` branch that was not made from a pull request (`is_pr_code`). Anything unknown is not trusted.
pub fn folder_trusted(dir: &Path) -> bool {
    current_branch(dir).is_ok_and(|b| b.starts_with("ax/")) && !is_pr_code(dir)
}

/// Whether `dir` holds a pull request's code: marked when `worktree.from_pr` made it, or still named `ax/pr-<number>`.
/// A folder that cannot be read counts as pull-request code.
pub fn is_pr_code(dir: &Path) -> bool {
    let Ok(branch) = current_branch(dir) else { return true };
    let numbered = branch.strip_prefix("ax/pr-").is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()));
    numbered
        || must(dir, &["rev-parse", "--path-format=absolute", "--git-dir"]).map_or(true, |gd| Path::new(&gd).join(UNTRUSTED_MARK).exists())
}

/// `git worktree list` is asked many times a second by the UI and the daemon (every folder check starts a process):
/// the answer is reused for a moment and dropped whenever a worktree is added or removed here.
type WtCache = std::collections::HashMap<String, (std::time::Instant, Vec<Worktree>)>;
static WT_CACHE: std::sync::Mutex<Option<WtCache>> = std::sync::Mutex::new(None);
/// Bumped by every `forget_worktree_lists`: a list computed before the bump may be stale and must not be cached.
static WT_GEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub fn forget_worktree_lists() {
    WT_GEN.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    *WT_CACHE.lock_recover() = None;
}

pub fn worktree_add(repo: &Path, path: &Path, branch: &str, base: &str) -> Result<()> {
    forget_worktree_lists();
    // checkout.workers=0: one checkout worker per core (git >= 2.32); on Windows this cut a 10k-file checkout from ~2.5 s to ~1 s
    // core.longpaths: deep trees would otherwise fail with "Filename too long" on Windows
    must(repo, &["-c", "checkout.workers=0", "-c", "core.longpaths=true", "worktree", "add", "-b", branch, &path.to_string_lossy(), base])?;
    Ok(())
}

/// Worktree removal often fails on Windows while a handle is still open,
/// so retry with backoff and finally park the directory in a trash folder.
pub fn worktree_remove(repo: &Path, path: &Path) -> Result<()> {
    forget_worktree_lists();
    // Fast path: renaming the folder is instant however big it is (node_modules, target/...), so the caller does not
    // wait for the files to be deleted. git forgets the worktree, and the folder is emptied in the background.
    if path.exists() {
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
        let trash =
            path.with_file_name(format!(".axelstra-trash-{}-{stamp}", path.file_name().map(|s| s.to_string_lossy()).unwrap_or_default()));
        if std::fs::rename(path, &trash).is_ok() {
            must(repo, &["worktree", "prune"])?;
            if let Some(dir) = trash.parent().map(Path::to_path_buf) {
                // kept: bounded background purge retries while a process still holds files in the trash folder
                std::thread::spawn(move || {
                    for wait in [0u64, 10, 10, 30, 60] {
                        sleep(Duration::from_secs(wait));
                        purge_trash(&dir);
                    }
                });
            }
            return Ok(());
        }
    }
    let p = path.to_string_lossy().to_string();
    for ms in [100u64, 300, 1000, 3000] {
        if run(repo, &["worktree", "remove", "--force", &p])?.ok {
            return Ok(());
        }
        // kept: bounded backoff for handles of a just-killed process (no OS event for "folder released")
        sleep(Duration::from_millis(ms));
    }
    // "Filename too long": git cannot walk a deep tree on Windows, remove_tree can
    if path.exists() && remove_tree(path).is_ok() {
        must(repo, &["worktree", "prune"])?;
        return Ok(());
    }
    if path.exists() {
        let trash = path.with_file_name(format!(".axelstra-trash-{}", path.file_name().map(|s| s.to_string_lossy()).unwrap_or_default()));
        std::fs::rename(path, &trash)
            .map_err(|e| coded!("worktree.park", "could not park {path}: {detail}", path = path.display(), detail = e))?;
        // the parked folder (node_modules and all) is deleted in the background once whatever held it lets go
        if let Some(dir) = trash.parent().map(Path::to_path_buf) {
            // kept: bounded background purge of the parked folder (6 x 10 s) while something holds it
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

/// All-or-nothing removal of a worktree for the janitor: the folder is renamed out of the way first (which Windows
/// refuses while any process has it, or something inside it, open), and only then does git forget the worktree.
/// When the rename keeps failing nothing was changed: git still lists the worktree and the folder is whole, so the
/// caller can try again once whatever holds it is gone. The renamed folder is deleted in the background.
pub fn worktree_remove_strict(repo: &Path, path: &Path) -> Result<()> {
    forget_worktree_lists();
    if !path.exists() {
        must(repo, &["worktree", "prune"])?;
        return Ok(());
    }
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    let trash =
        path.with_file_name(format!(".axelstra-trash-{}-{stamp}", path.file_name().map(|s| s.to_string_lossy()).unwrap_or_default()));
    let mut last = None;
    // the handles of a process that was just killed take a moment to go
    // kept: bounded backoff, Windows has no event for "last handle on the folder closed"
    for ms in [0u64, 100, 300, 1000] {
        sleep(Duration::from_millis(ms));
        match std::fs::rename(path, &trash) {
            Ok(()) => {
                last = None;
                break;
            }
            Err(e) => last = Some(e),
        }
    }
    if let Some(e) = last {
        return Err(coded!(
            "worktree.folder_busy",
            "the folder {path} is still in use by a process, so the worktree was left as it was: {detail}",
            path = path.display(),
            detail = e
        ));
    }
    if let Err(e) = must(repo, &["worktree", "prune"]) {
        // git did not forget it: put the folder back so git and disk agree
        let _ = std::fs::rename(&trash, path);
        return Err(e);
    }
    if let Some(dir) = trash.parent().map(Path::to_path_buf) {
        // kept: same bounded background purge as in `worktree_remove`
        std::thread::spawn(move || {
            for wait in [0u64, 10, 10, 30, 60] {
                sleep(Duration::from_secs(wait));
                purge_trash(&dir);
            }
        });
    }
    Ok(())
}

/// Deletes the `.axelstra-trash-*` folders left in `dir` by removals that had to park a worktree.
pub fn purge_trash(dir: &Path) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        if e.file_name().to_string_lossy().starts_with(".axelstra-trash-") {
            let _ = remove_tree(&e.path());
        }
    }
}

/// `path` in the verbatim form (`\\?\C:\...`) on Windows, which lifts the 260-character limit that deep `node_modules`
/// and `target` trees run into ("Filename too long"). Elsewhere, or when the path cannot be made absolute, unchanged.
pub fn verbatim(path: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        let s = path.to_string_lossy();
        if !s.starts_with(r"\\?\") {
            if let Ok(abs) = std::path::absolute(path) {
                let a = abs.to_string_lossy().replace('/', r"\");
                return PathBuf::from(match a.strip_prefix(r"\\") {
                    Some(unc) => format!(r"\\?\UNC\{unc}"),
                    None => format!(r"\\?\{a}"),
                });
            }
        }
    }
    path.to_path_buf()
}

/// Deletes a folder tree however deep its paths are. Tries the verbatim path first; on Windows, if that fails, empties
/// the tree by mirroring an empty folder over it with `robocopy /MIR` (which ignores path length) and removes the husk.
pub fn remove_tree(path: &Path) -> std::io::Result<()> {
    if std::fs::symlink_metadata(verbatim(path)).is_err() {
        return Ok(());
    }
    let first = std::fs::remove_dir_all(verbatim(path));
    if first.is_ok() || !cfg!(windows) {
        return first;
    }
    let empty = path.with_file_name(format!(".axelstra-empty-{}", std::process::id()));
    std::fs::create_dir_all(&empty)?;
    // exit codes 0-7 are some kind of success, 8 and up are failures
    let _ = Command::new("robocopy")
        .arg(&empty)
        .arg(path)
        .args(["/MIR", "/NFL", "/NDL", "/NJH", "/NJS", "/NP", "/R:0", "/W:0"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    let _ = std::fs::remove_dir(&empty);
    match std::fs::remove_dir_all(verbatim(path)) {
        Ok(()) => Ok(()),
        Err(_) if std::fs::symlink_metadata(verbatim(path)).is_err() => Ok(()),
        Err(e) => Err(e),
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
    must(wt, &["-c", "user.name=axelstra", "-c", "user.email=axelstra@localhost", "commit", "--no-verify", "-m", msg])?;
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
        &["-c", "user.name=axelstra", "-c", "user.email=axelstra@localhost", "merge", "--no-ff", "--no-verify", "-m", msg, branch],
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
    use std::sync::atomic::Ordering;
    let key = repo.to_string_lossy().to_string();
    if let Some((at, v)) = WT_CACHE.lock_recover().as_ref().and_then(|m| m.get(&key)) {
        if at.elapsed() < Duration::from_secs(3) {
            return Ok(v.clone());
        }
    }
    let generation = WT_GEN.load(Ordering::SeqCst);
    let v = worktree_list_uncached(repo)?;
    let mut cache = WT_CACHE.lock_recover();
    // a worktree was added or removed while git ran: this answer may predate it, so it is returned but not kept
    if WT_GEN.load(Ordering::SeqCst) == generation {
        cache.get_or_insert_with(Default::default).insert(key, (std::time::Instant::now(), v.clone()));
    }
    Ok(v)
}

/// True when `path` is inside a git repository (a plain folder has no `.git` to find).
pub fn is_repo(path: &Path) -> bool {
    run(path, &["rev-parse", "--git-dir"]).map(|o| o.ok).unwrap_or(false)
}

fn worktree_list_uncached(repo: &Path) -> Result<Vec<Worktree>> {
    // a project that is just a folder (no git yet) is its own single "main worktree": agents and terminals run in it
    if !is_repo(repo) {
        return Ok(vec![Worktree { path: repo.to_string_lossy().to_string(), branch: None, head: String::new(), is_main: true }]);
    }
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

#[cfg(test)]
mod run_tests {
    use super::*;

    #[test]
    fn git_runs_with_prompts_disabled_and_a_deadline() {
        let o = run(Path::new("."), &["--version"]).unwrap();
        assert!(o.ok && o.stdout.starts_with("git version"));
        // an alias that never ends stands in for a fetch stuck on a credential prompt
        let sleeper = "!sleep 60"; // aliases run in git's own sh (Git for Windows ships sleep)
        let t = std::time::Instant::now();
        let e = run_timeout(Path::new("."), &["-c", &format!("alias.hang={sleeper}"), "hang"], Duration::from_millis(500)).unwrap_err();
        assert!(e.to_string().contains("timed out"), "{e}");
        assert!(t.elapsed() < Duration::from_secs(15));
    }

    #[test]
    fn prompt_env_is_set() {
        let sh = "!echo $GIT_TERMINAL_PROMPT/$GCM_INTERACTIVE"; // aliases run in git's own sh
        let o = run(Path::new("."), &["-c", &format!("alias.axenv={sh}"), "axenv"]).unwrap();
        assert!(o.stdout.contains("0/never"), "{}", o.stdout);
    }

    #[test]
    fn forgetting_during_a_listing_does_not_cache_the_old_answer() {
        let dir = std::env::temp_dir().join(format!("axelstra-wtcache-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let _ = must(&dir, &["init", "-q"]);
        let a = worktree_list(&dir).unwrap();
        forget_worktree_lists();
        // after a forget the next call must ask git again, not serve the cached list
        let key = dir.to_string_lossy().to_string();
        assert!(WT_CACHE.lock_recover().as_ref().is_none_or(|m| !m.contains_key(&key)));
        assert_eq!(a.len(), worktree_list(&dir).unwrap().len());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod failure_tests {
    use super::*;

    #[test]
    fn stderr_picks_the_usual_causes() {
        let c = |s: &str| classify_failure(s);
        assert_eq!(c("fatal: not a git repository (or any parent)"), Some("git.not_repo"));
        assert_eq!(c("error: Your local changes would be overwritten by merge"), Some("git.dirty"));
        assert_eq!(c("fatal: 'x' is already checked out at 'y'"), Some("git.exists"));
        assert_eq!(c("fatal: Could not read from remote repository."), Some("git.remote"));
        assert_eq!(c(" ! [rejected] main -> main (non-fast-forward)"), Some("git.rejected"));
        assert_eq!(c("CONFLICT (content): Merge conflict in a.rs"), Some("git.conflicts"));
        assert_eq!(c("nothing to commit, working tree clean"), Some("git.nothing_to_commit"));
        assert_eq!(c("fatal: bad revision 'zz'"), Some("git.bad_ref"));
        assert_eq!(c("something else"), None);
    }

    #[test]
    fn failure_keeps_git_words_and_carries_the_code() {
        let e = failure(&["push", "origin"], "fatal: not a git repository");
        assert_eq!(format!("{e}"), "git push origin failed: fatal: not a git repository");
        let c = crate::msg::find(&e).unwrap();
        assert_eq!(c.code, "git.not_repo");
        assert_eq!(c.params["cmd"], "push origin");
        assert_eq!(crate::msg::find(&failure(&["x"], "weird")).unwrap().code, "git.failed");
    }
}

#[cfg(test)]
mod long_path_tests {
    use super::*;

    /// A tree whose deepest path is far past 260 characters (what `node_modules` and `target` produce).
    fn deep(root: &Path) -> PathBuf {
        let mut d = verbatim(root);
        for i in 0..12 {
            d.push(format!("level-{i:02}-{}", "x".repeat(40)));
        }
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("leaf.txt"), "x").unwrap();
        d
    }

    #[test]
    fn remove_tree_deletes_paths_past_the_windows_limit() {
        let root = std::env::temp_dir().join(format!("axl-long-{}", std::process::id()));
        let leaf = deep(&root);
        assert!(leaf.to_string_lossy().len() > 260 + 4);
        remove_tree(&root).unwrap();
        assert!(!root.exists());
        remove_tree(&root).unwrap(); // already gone is fine
    }

    #[test]
    fn a_worktree_with_a_deep_tree_is_removed() {
        let root = std::env::temp_dir().join(format!("axl-longwt-{}", std::process::id()));
        let _ = remove_tree(&root);
        let repo = root.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        must(&repo, &["init", "-q", "-b", "main"]).unwrap();
        must(&repo, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "--allow-empty", "-m", "i"]).unwrap();
        let wt = root.join("wt");
        worktree_add(&repo, &wt, "ax/deep", "HEAD").unwrap();
        deep(&wt.join("node_modules"));
        worktree_remove(&repo, &wt).unwrap();
        assert!(!wt.exists());
        // the parked folder is emptied by the background purge, also when it is deep
        purge_trash(&root);
        let left =
            std::fs::read_dir(&root).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().starts_with(".axelstra-trash-")).count();
        assert_eq!(left, 0);
        assert!(!worktree_list(&repo).unwrap().iter().any(|w| w.branch.as_deref() == Some("ax/deep")));
        let _ = remove_tree(&root);
    }
}
