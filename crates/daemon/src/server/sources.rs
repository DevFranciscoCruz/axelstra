//! The sources of the Add Project modal: `project.discover` / `project.add_discovered` (repositories already on disk),
//! `project.clone*` (a background `git clone`), `project.init` (a new repository) and `gh.repos` (the user's GitHub
//! repositories). All of them are the user's: an agent connection is refused (`agent_forbidden`). Nothing here asks for
//! or stores a credential: git and gh use what the user already set up, and git never prompts.
use super::*;
use axelstra_core::job::Job;
use axelstra_core::msg::Coded;
use axelstra_core::{coded, git, msg, paths};
use std::collections::HashSet;
use std::sync::Condvar;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

impl Registry {
    /// The clone `id` moved (progress, or its end): the app reads `project.clone_status` then instead of asking on a timer.
    pub(super) fn clone_changed(&self, id: &str) {
        self.broadcast_event(json!({"event": "clone.changed", "id": id}));
    }
}

pub(super) fn handles(method: &str) -> bool {
    matches!(
        method,
        "project.discover"
            | "project.add_discovered"
            | "project.clone"
            | "project.clone_status"
            | "project.clone_cancel"
            | "project.init"
            | "gh.repos"
    )
}

pub(super) fn call(c: &mut Ctx) -> Result<Value> {
    let (p, reg) = (c.p, c.reg);
    match c.req.method.as_str() {
        "project.discover" => Ok(tokio::task::block_in_place(|| discover(&reg.store))),
        "project.add_discovered" => {
            let path = p["path"].as_str().ok_or_else(|| missing("path"))?;
            let key = paths::norm(path);
            if !DISCOVERED.lock_recover().contains(&key) {
                return Err(coded!("project.not_discovered", "{path} was not offered by project.discover", path = path));
            }
            let pr = tokio::task::block_in_place(|| crate::projects::add(&reg.store, path))?;
            DISCOVERED.lock_recover().remove(&key);
            Ok(serde_json::to_value(pr)?)
        }
        "project.clone" => tokio::task::block_in_place(|| start_clone(reg, p)),
        "project.clone_status" => clone_status(p["id"].as_str().ok_or_else(|| missing("id"))?),
        "project.clone_cancel" => tokio::task::block_in_place(|| clone_cancel(p["id"].as_str().ok_or_else(|| missing("id"))?)),
        "project.init" => tokio::task::block_in_place(|| init(reg, p)),
        "gh.repos" => tokio::task::block_in_place(|| gh_repos(p)),
        m => Err(unknown_method(m)),
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// project.discover

/// Folders of the home directory where people keep repositories.
const DISCOVER_ROOTS: [&str; 7] = ["Projects", "Projetos", "Documents/GitHub", "source/repos", "dev", "code", "Desktop"];
/// How long a scan may take, and the most repositories it returns.
const DISCOVER_BUDGET: Duration = Duration::from_secs(3);
const DISCOVER_MAX: usize = 200;
/// A repository touched in the last days is offered first, as `recent`.
const RECENT_SECS: u64 = 30 * 24 * 3600;
/// Folders that are never worth descending into.
const SKIP_DIRS: [&str; 4] = ["node_modules", "target", "vendor", "$recycle.bin"];

/// Normalized paths the last `project.discover` returned: `project.add_discovered` takes nothing else, so a page cannot
/// register an arbitrary folder through it.
static DISCOVERED: std::sync::LazyLock<Mutex<HashSet<String>>> = std::sync::LazyLock::new(Default::default);

struct Found {
    path: PathBuf,
    name: String,
    branch: Option<String>,
    touched: u64,
}

/// The branch `HEAD` names (a detached head has none).
fn head_branch(repo: &Path) -> Option<String> {
    let head = std::fs::read_to_string(repo.join(".git").join("HEAD")).ok()?;
    head.trim().strip_prefix("ref: refs/heads/").map(String::from)
}

/// Seconds since the epoch of the last change to `.git/HEAD` (0 when unknown).
fn touched(repo: &Path) -> u64 {
    std::fs::metadata(repo.join(".git").join("HEAD"))
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs())
}

/// Git repositories under `roots`, at most two levels down (`root/a`, `root/a/b`), not in `known`. A repository is not
/// searched for more repositories; hidden folders and `SKIP_DIRS` are skipped. Stops at `deadline` or `max` results.
fn scan(roots: &[PathBuf], known: &HashSet<String>, deadline: Instant, max: usize) -> Vec<Found> {
    fn visit(
        dir: &Path,
        depth: u8,
        known: &HashSet<String>,
        seen: &mut HashSet<String>,
        out: &mut Vec<Found>,
        deadline: Instant,
        max: usize,
    ) {
        if out.len() >= max || Instant::now() > deadline {
            return;
        }
        if depth > 0 && dir.join(".git").is_dir() {
            let key = paths::norm_path(dir);
            if !known.contains(&key) && !crate::projects::forbidden_root(dir) && seen.insert(key) {
                let name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                out.push(Found { path: dir.to_path_buf(), name, branch: head_branch(dir), touched: touched(dir) });
            }
            return;
        }
        if depth >= 2 {
            return;
        }
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let lower = name.to_ascii_lowercase();
            // symlinks are not followed: a loop or a link out of the tree costs the whole budget
            if name.starts_with('.') || SKIP_DIRS.contains(&lower.as_str()) || !e.file_type().is_ok_and(|t| t.is_dir()) {
                continue;
            }
            visit(&e.path(), depth + 1, known, seen, out, deadline, max);
        }
    }
    let (mut out, mut seen, mut roots_seen) = (Vec::new(), HashSet::new(), HashSet::new());
    for r in roots {
        if roots_seen.insert(paths::norm_path(r)) && r.is_dir() {
            visit(r, 0, known, &mut seen, &mut out, deadline, max);
        }
    }
    out
}

fn discover(store: &Path) -> Value {
    let Some(home) = crate::hosts::home() else { return json!({"items": []}) };
    let roots: Vec<PathBuf> = DISCOVER_ROOTS.iter().map(|r| home.join(r)).collect();
    let known: HashSet<String> = crate::projects::load(store).iter().map(|p| paths::norm(&p.path)).collect();
    let mut found = scan(&roots, &known, Instant::now() + DISCOVER_BUDGET, DISCOVER_MAX);
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let recent = |f: &Found| f.touched > 0 && now.saturating_sub(f.touched) <= RECENT_SECS;
    found.sort_by(|a, b| {
        recent(b).cmp(&recent(a)).then_with(|| {
            if recent(a) {
                b.touched.cmp(&a.touched)
            } else {
                a.name.to_lowercase().cmp(&b.name.to_lowercase())
            }
        })
    });
    *DISCOVERED.lock_recover() = found.iter().map(|f| paths::norm_path(&f.path)).collect();
    let items: Vec<Value> = found
        .iter()
        .map(|f| {
            json!({"path": crate::hosts::display(&f.path), "name": f.name, "branch": f.branch,
                "source": if recent(f) { "recent" } else { "scan" }})
        })
        .collect();
    json!({"items": items})
}

// ---------------------------------------------------------------------------------------------------------------------
// project.clone

const MAX_RUNNING_CLONES: usize = 4;
/// A finished clone stays readable by `project.clone_status` this long.
const KEEP_FINISHED: Duration = Duration::from_secs(600);
const MAX_URL: usize = 2000;
/// Lines of git's own output kept to explain a failure.
const TAIL_LINES: usize = 12;

/// A URL git may clone: https, ssh, `user@host:path` and, only under test, file. Anything else (a leading `-`, `ext::`
/// helpers, spaces, control characters) is refused before git sees it.
fn check_url(url: &str, allow_file: bool) -> Result<String> {
    let url = url.trim();
    if url.is_empty() {
        return Err(coded!("project.clone_url_empty", "the repository URL is empty"));
    }
    let bad = || {
        coded!(
            "project.clone_url_invalid",
            "{url} is not a repository URL git may clone (use https://, ssh:// or git@host:path)",
            url = url
        )
    };
    if url.len() > MAX_URL || url.starts_with('-') || url.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(bad());
    }
    let lower = url.to_ascii_lowercase();
    let after = |prefix: &str| lower.strip_prefix(prefix).filter(|r| !r.is_empty() && !r.starts_with('/'));
    if after("https://").is_some() || after("ssh://").is_some() || (allow_file && lower.starts_with("file://")) {
        return Ok(url.to_string());
    }
    // scp-like: user@host:path
    if !url.contains("://") {
        if let Some((user, rest)) = url.split_once('@') {
            if let Some((host, path)) = rest.split_once(':') {
                let user_ok = !user.is_empty() && user.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
                let host_ok = !host.is_empty() && host.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-'));
                if user_ok && host_ok && !path.is_empty() {
                    return Ok(url.to_string());
                }
            }
        }
    }
    Err(bad())
}

fn check_branch(branch: &str) -> Result<String> {
    let b = branch.trim();
    let bad = b.is_empty()
        || b.len() > 200
        || b.starts_with('-')
        || b.contains("..")
        || b.chars().any(|c| c.is_whitespace() || c.is_control() || matches!(c, '~' | '^' | ':' | '?' | '*' | '[' | '\\'));
    if bad {
        return Err(coded!("project.clone_branch_invalid", "{branch} is not a valid branch name", branch = b));
    }
    Ok(b.to_string())
}

/// The destination folder and whether the clone creates it (so a failed or cancelled clone removes it again). It must
/// not exist or be an empty folder.
fn check_dest(dest: &str) -> Result<(PathBuf, bool)> {
    let d = PathBuf::from(dest.trim());
    if dest.trim().is_empty() || dest.contains('\0') || !d.is_absolute() {
        return Err(coded!("project.clone_dest_relative", "{path} is not an absolute path", path = dest.trim()));
    }
    if crate::projects::forbidden_root(&d) {
        return Err(coded!(
            "project.forbidden_root",
            "{path} cannot be a project: it is a filesystem root, the home folder or a system folder",
            path = d.display()
        ));
    }
    match std::fs::read_dir(&d) {
        Ok(mut rd) => {
            if rd.next().is_some() {
                return Err(coded!("project.clone_dest_exists", "{path} already exists and is not empty", path = d.display()));
            }
            Ok((d, false))
        }
        Err(_) if d.exists() => Err(coded!("project.clone_dest_exists", "{path} already exists and is not empty", path = d.display())),
        Err(_) => Ok((d, true)),
    }
}

/// `(phase, percent)` of a progress line git prints with `--progress` ("Receiving objects:  45% (45/100), ...").
fn parse_progress(line: &str) -> Option<(String, u32)> {
    let line = line.trim().trim_start_matches("remote:").trim();
    let (phase, rest) = line.split_once(':')?;
    let pct: String = rest.trim_start().chars().take_while(char::is_ascii_digit).collect();
    if pct.is_empty() || !rest.trim_start()[pct.len()..].starts_with('%') {
        return None;
    }
    Some((phase.trim().to_string(), pct.parse::<u32>().ok()?.min(100)))
}

/// Overall percent of the whole clone: receiving is the long part, resolving the rest. Phases before it carry no number.
fn overall(phase: &str, pct: u32) -> Option<u32> {
    match phase {
        "Receiving objects" => Some(pct * 8 / 10),
        "Resolving deltas" => Some(80 + pct / 5),
        "Updating files" | "Filtering content" => Some(99),
        _ => None,
    }
}

#[derive(Clone, Default)]
struct Status {
    state: &'static str,
    percent: Option<u32>,
    phase: Option<String>,
    path: Option<String>,
    project: Option<Value>,
    error: Option<String>,
    code: Option<Coded>,
    finished: Option<Instant>,
}

struct CloneJob {
    dest: PathBuf,
    /// the clone made the folder: removing it on failure or cancel is then safe
    created: bool,
    job: Job,
    cancelled: AtomicBool,
    status: Mutex<Status>,
    /// signalled by `finish`, so a cancel waits for the end of the clone instead of looking again and again
    ended: Condvar,
}

static CLONES: Mutex<Vec<(String, Arc<CloneJob>)>> = Mutex::new(Vec::new());
static CLONE_SEQ: AtomicU64 = AtomicU64::new(1);

impl CloneJob {
    fn running(&self) -> bool {
        self.status.lock_recover().state == "running"
    }

    fn finish(&self, f: impl FnOnce(&mut Status)) {
        let mut s = self.status.lock_recover();
        f(&mut s);
        s.finished = Some(Instant::now());
        self.ended.notify_all();
    }

    fn fail(&self, e: &anyhow::Error) {
        self.finish(|s| {
            s.state = "failed";
            s.error = Some(format!("{e:#}"));
            s.code = msg::find(e).cloned();
        });
    }
}

/// Removes what a failed or cancelled clone left: the folder it made, or the contents of the empty folder it was given.
fn cleanup(j: &CloneJob) {
    for _ in 0..20 {
        let done = if j.created {
            std::fs::remove_dir_all(&j.dest)
        } else {
            std::fs::read_dir(&j.dest).map(|rd| {
                for e in rd.flatten() {
                    let _ = if e.path().is_dir() { std::fs::remove_dir_all(e.path()) } else { std::fs::remove_file(e.path()) };
                }
            })
        };
        // the dying git may still hold a file for a moment
        if done.is_ok() || !j.dest.exists() {
            return;
        }
        // kept: bounded retry (20 x 150 ms) for a file the dying git or an antivirus still holds; no OS event for it
        std::thread::sleep(Duration::from_millis(150));
    }
}

/// The error for a clone that exited non-zero, from the lines git printed.
fn clone_error(tail: &[String], exit: Option<i32>) -> anyhow::Error {
    let detail =
        if tail.is_empty() { format!("git exited with {}", exit.map_or("no code".into(), |c| c.to_string())) } else { tail.join("\n") };
    let detail: String = detail.chars().take(600).collect();
    let low = detail.to_lowercase();
    let auth = [
        "authentication failed",
        "could not read username",
        "could not read password",
        "terminal prompts disabled",
        "permission denied",
        "repository not found",
        "access denied",
        "host key verification failed",
    ];
    if auth.iter().any(|a| low.contains(a)) {
        coded!("project.clone_auth", "git clone could not authenticate: {detail}", detail = detail)
    } else {
        coded!("project.clone_failed", "git clone failed: {detail}", detail = detail)
    }
}

/// Reads git's stderr (progress lines end in `\r`, the rest in `\n`) into the job's status.
/// `moved` runs when the visible progress (percent or phase) changed, so the app reads the status then.
fn read_progress(j: &CloneJob, mut err: impl Read, tail: &mut Vec<String>, moved: &dyn Fn()) {
    let (mut buf, mut chunk) = (Vec::<u8>::new(), [0u8; 4096]);
    let mut line = |bytes: &[u8]| {
        let text = String::from_utf8_lossy(bytes).trim().to_string();
        if text.is_empty() {
            return;
        }
        match parse_progress(&text) {
            Some((phase, pct)) => {
                let changed = {
                    let mut s = j.status.lock_recover();
                    let before = (s.percent, s.phase.clone());
                    if let Some(o) = overall(&phase, pct) {
                        s.percent = Some(s.percent.map_or(o, |p| p.max(o)));
                    }
                    s.phase = Some(phase);
                    before != (s.percent, s.phase.clone())
                };
                if changed {
                    moved();
                }
            }
            None => {
                tail.push(text);
                if tail.len() > TAIL_LINES {
                    tail.remove(0);
                }
            }
        }
    };
    loop {
        match err.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                while let Some(i) = buf.iter().position(|b| *b == b'\r' || *b == b'\n') {
                    let rest = buf.split_off(i + 1);
                    line(&buf[..i]);
                    buf = rest;
                }
                // a line that never ends is not progress
                if buf.len() > 64 << 10 {
                    buf.clear();
                }
            }
        }
    }
    line(&buf);
}

fn start_clone(reg: &Arc<Registry>, p: &Value) -> Result<Value> {
    let url = check_url(p["url"].as_str().ok_or_else(|| missing("url"))?, crate::proto::testing())?;
    let (dest, created) = check_dest(p["dest"].as_str().ok_or_else(|| missing("dest"))?)?;
    let branch = match p["branch"].as_str().map(str::trim).filter(|b| !b.is_empty()) {
        Some(b) => Some(check_branch(b)?),
        None => None,
    };
    let depth = match &p["depth"] {
        Value::Null => None,
        v => match v.as_u64().filter(|d| (1..=1_000_000).contains(d)) {
            Some(d) => Some(d),
            None => return Err(coded!("project.clone_depth_invalid", "depth must be a whole number from 1 to 1000000")),
        },
    };
    let key = paths::norm_path(&dest);
    let job = {
        let mut jobs = CLONES.lock_recover();
        jobs.retain(|(_, j)| j.running() || j.status.lock_recover().finished.is_some_and(|f| f.elapsed() < KEEP_FINISHED));
        let live: Vec<_> = jobs.iter().filter(|(_, j)| j.running()).collect();
        if live.iter().any(|(_, j)| paths::norm_path(&j.dest) == key) {
            return Err(coded!("project.clone_dest_busy", "a clone into {path} is already running", path = dest.display()));
        }
        if live.len() >= MAX_RUNNING_CLONES {
            return Err(coded!(
                "project.clone_too_many",
                "{max} clones are already running; wait for one to finish",
                max = MAX_RUNNING_CLONES
            ));
        }
        let j = Arc::new(CloneJob {
            dest: dest.clone(),
            created,
            job: Job::new()?,
            cancelled: AtomicBool::new(false),
            status: Mutex::new(Status { state: "running", ..Default::default() }),
            ended: Condvar::new(),
        });
        let id = format!("clone-{}", CLONE_SEQ.fetch_add(1, Ordering::Relaxed));
        jobs.push((id.clone(), j.clone()));
        (id, j)
    };
    let (id, j) = job;
    let mut cmd = Command::new("git");
    // never a prompt: a missing credential fails the clone, it does not wait for a terminal nobody sees
    cmd.env("GIT_TERMINAL_PROMPT", "0").env("GCM_INTERACTIVE", "never").args(["-c", "protocol.ext.allow=never"]);
    if cfg!(windows) {
        cmd.args(["-c", "core.longpaths=true"]);
    }
    cmd.args(["clone", "--progress"]);
    if let Some(b) = &branch {
        cmd.args(["--branch", b]);
    }
    if let Some(d) = depth {
        cmd.args(["--depth", &d.to_string()]);
    }
    cmd.arg("--").arg(&url).arg(&dest).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped());
    // ssh must not ask for a passphrase or a host-key answer either; the user's own core.sshCommand wins
    if !url.to_ascii_lowercase().starts_with("https://")
        && std::env::var_os("GIT_SSH_COMMAND").is_none()
        && git::run(&std::env::temp_dir(), &["config", "--get", "core.sshCommand"]).is_ok_and(|o| o.stdout.is_empty())
    {
        cmd.env("GIT_SSH_COMMAND", "ssh -o BatchMode=yes");
    }
    let mut child = match j.job.spawn(&mut cmd) {
        Ok(c) => c,
        Err(e) => {
            let err = coded!("project.clone_spawn", "could not start git clone: {detail}", detail = format!("{e:#}"));
            j.fail(&err);
            return Err(err);
        }
    };
    let (reg, jj, cid) = (reg.clone(), j.clone(), id.clone());
    std::thread::spawn(move || {
        let announce = || reg.clone_changed(&cid);
        let mut tail = Vec::new();
        if let Some(err) = child.stderr.take() {
            read_progress(&jj, err, &mut tail, &announce);
        }
        let exit = child.wait();
        #[cfg(unix)]
        jj.job.forget(child.id());
        if jj.cancelled.load(Ordering::SeqCst) {
            cleanup(&jj);
            jj.finish(|s| s.state = "cancelled");
            announce();
            return;
        }
        match exit {
            Ok(st) if st.success() => match crate::projects::add(&reg.store, &jj.dest.to_string_lossy()) {
                Ok(pr) => {
                    let (path, project) = (pr.path.clone(), serde_json::to_value(&pr).ok());
                    jj.finish(|s| {
                        s.state = "done";
                        s.percent = Some(100);
                        s.path = Some(path);
                        s.project = project;
                    });
                    reg.projects_changed();
                }
                Err(e) => jj.fail(&e),
            },
            other => {
                cleanup(&jj);
                jj.fail(&clone_error(&tail, other.ok().and_then(|s| s.code())));
            }
        }
        announce();
    });
    Ok(json!({"id": id}))
}

fn find_clone(id: &str) -> Result<Arc<CloneJob>> {
    CLONES
        .lock_recover()
        .iter()
        .find(|(i, _)| i == id)
        .map(|(_, j)| j.clone())
        .ok_or_else(|| coded!("project.clone_unknown", "no clone {id}", id = id))
}

fn clone_status(id: &str) -> Result<Value> {
    let s = find_clone(id)?.status.lock_recover().clone();
    let mut v = json!({"state": s.state});
    for (k, x) in [
        ("percent", json!(s.percent)),
        ("phase", json!(s.phase)),
        ("path", json!(s.path)),
        ("project", json!(s.project)),
        ("error", json!(s.error)),
    ] {
        if !x.is_null() {
            v[k] = x;
        }
    }
    if let Some(c) = s.code {
        v["error_code"] = json!(c.code);
        v["error_params"] = c.params_json();
    }
    Ok(v)
}

/// Kills the clone's process tree and waits (a few seconds at most) until its partial folder is gone.
fn clone_cancel(id: &str) -> Result<Value> {
    let j = find_clone(id)?;
    if j.running() {
        j.cancelled.store(true, Ordering::SeqCst);
        j.job.kill();
        let g = j.status.lock_recover();
        let _ = j.ended.wait_timeout_while(g, Duration::from_secs(10), |s| s.state == "running");
    }
    Ok(json!({"ok": true}))
}

// ---------------------------------------------------------------------------------------------------------------------
// project.init

/// A folder name that is valid on every platform: one segment, no separators, no `..`, no reserved Windows name.
fn check_name(name: &str) -> Result<String> {
    let bad = || coded!("project.init_name_invalid", "{name} is not a valid folder name", name = name);
    let reserved = |n: &str| {
        let stem = n.split('.').next().unwrap_or_default().to_ascii_uppercase();
        matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || (stem.len() == 4
                && (stem.starts_with("COM") || stem.starts_with("LPT"))
                && stem.ends_with(|c: char| ('1'..='9').contains(&c)))
    };
    if name.is_empty()
        || name != name.trim()
        || name.chars().count() > 100
        || name == "."
        || name == ".."
        || name.starts_with('-')
        || name.ends_with('.')
        || name.chars().any(|c| c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'))
        || reserved(name)
    {
        return Err(bad());
    }
    Ok(name.to_string())
}

const GITIGNORE: &str = "# OS\n.DS_Store\nThumbs.db\n\n# Dependencies and build output\nnode_modules/\ntarget/\ndist/\nbuild/\n\n# Local environment\n.env\n.env.*\n!.env.example\n";

fn init_failed(detail: impl std::fmt::Display) -> anyhow::Error {
    coded!("project.init_failed", "could not create the repository: {detail}", detail = detail)
}

fn init(reg: &Arc<Registry>, p: &Value) -> Result<Value> {
    let name = check_name(p["name"].as_str().ok_or_else(|| missing("name"))?)?;
    let parent = PathBuf::from(p["parent"].as_str().ok_or_else(|| missing("parent"))?.trim());
    if !parent.is_absolute() || !parent.is_dir() {
        return Err(coded!("project.init_parent_missing", "{path} is not an existing folder", path = parent.display()));
    }
    let dir = parent.join(&name);
    if crate::projects::forbidden_root(&dir) {
        return Err(coded!(
            "project.forbidden_root",
            "{path} cannot be a project: it is a filesystem root, the home folder or a system folder",
            path = dir.display()
        ));
    }
    let created = match std::fs::read_dir(&dir).map(|mut rd| rd.next().is_none()) {
        Ok(true) => false,
        Ok(false) => return Err(coded!("project.init_exists", "{path} already exists and is not empty", path = dir.display())),
        Err(_) if dir.exists() => {
            return Err(coded!("project.init_exists", "{path} already exists and is not empty", path = dir.display()))
        }
        Err(_) => {
            std::fs::create_dir(&dir).map_err(init_failed)?;
            true
        }
    };
    let flag = |k: &str| p[k].as_bool().unwrap_or(false);
    let built = (|| -> Result<Vec<&'static str>> {
        git::must(&dir, &["init"]).map_err(|e| init_failed(format!("{e:#}")))?;
        let mut files = Vec::new();
        if flag("readme") {
            std::fs::write(dir.join("README.md"), format!("# {name}\n")).map_err(init_failed)?;
            files.push("README.md");
        }
        if flag("gitignore") {
            std::fs::write(dir.join(".gitignore"), GITIGNORE).map_err(init_failed)?;
            files.push(".gitignore");
        }
        Ok(files)
    })();
    let files = match built {
        Ok(f) => f,
        Err(e) => {
            if created {
                let _ = std::fs::remove_dir_all(&dir);
            }
            return Err(e);
        }
    };
    // registered before the commit: a missing git identity leaves a usable repository, only the first commit is not made
    let pr = crate::projects::add(&reg.store, &dir.to_string_lossy())?;
    if flag("commit") {
        let has = |k: &str| git::run(&dir, &["config", k]).is_ok_and(|o| o.ok && !o.stdout.is_empty());
        if !(has("user.name") && has("user.email")) {
            return Err(coded!(
                "project.init_no_identity",
                "the repository was created, but git has no name and email to sign the first commit (git config --global user.name / user.email)"
            ));
        }
        if !files.is_empty() {
            let mut add = vec!["add", "--"];
            add.extend(files.iter());
            git::must(&dir, &add).map_err(|e| init_failed(format!("{e:#}")))?;
        }
        git::must(&dir, &["commit", "--allow-empty", "-m", "Initial commit"]).map_err(|e| init_failed(format!("{e:#}")))?;
    }
    Ok(serde_json::to_value(pr)?)
}

// ---------------------------------------------------------------------------------------------------------------------
// gh.repos

/// Repositories from `gh repo list --json ...`, filtered by `query` (lowercase; a substring of the name or description)
/// and cut to `limit`.
fn parse_repos(json: &str, query: &str, limit: usize) -> Result<Vec<Value>> {
    let v: Value = serde_json::from_str(json).map_err(|_| coded!("gh.bad_json", "gh returned invalid JSON"))?;
    let s = |r: &Value, k: &str| r[k].as_str().unwrap_or_default().to_string();
    Ok(v.as_array()
        .map(|a| a.as_slice())
        .unwrap_or_default()
        .iter()
        .filter(|r| {
            query.is_empty() || s(r, "nameWithOwner").to_lowercase().contains(query) || s(r, "description").to_lowercase().contains(query)
        })
        .take(limit)
        .map(|r| {
            json!({"name_with_owner": s(r, "nameWithOwner"), "description": s(r, "description"),
                "is_private": r["isPrivate"].as_bool().unwrap_or(false), "url": s(r, "url"), "pushed_at": s(r, "pushedAt")})
        })
        .collect())
}

fn gh_repos(p: &Value) -> Result<Value> {
    let limit = usize::try_from(p["limit"].as_u64().unwrap_or(50)).unwrap_or(50).clamp(1, 200);
    let query = p["query"].as_str().unwrap_or_default().trim().to_lowercase();
    // gh has no search on `repo list`: with a query the most it lists is fetched and filtered here
    let fetch = if query.is_empty() { limit } else { 200 };
    let mut cmd = Command::new("gh");
    cmd.env("GH_PROMPT_DISABLED", "1").args([
        "repo",
        "list",
        "--json",
        "nameWithOwner,description,isPrivate,url,pushedAt",
        "--limit",
        &fetch.to_string(),
    ]);
    git::quiet(&mut cmd);
    let o = match axelstra_core::exec::run_capped(&mut cmd, Duration::from_secs(30), 4 << 20) {
        Ok(o) => o,
        Err(_) => return Ok(json!({"state": "no_gh", "repos": []})),
    };
    if o.timed_out {
        return Err(coded!("gh.repos_failed", "gh repo list failed: {detail}", detail = "timed out after 30s"));
    }
    if !o.success() {
        let err = o.stderr_lossy();
        if crate::checks::classify_failure(&err) == "auth" {
            return Ok(json!({"state": "not_authed", "repos": []}));
        }
        return Err(coded!("gh.repos_failed", "gh repo list failed: {detail}", detail = err.trim()));
    }
    Ok(json!({"state": "ok", "repos": parse_repos(&o.stdout_lossy(), &query, limit)?}))
}

#[cfg(test)]
mod tests {
    #[test]
    fn finishing_a_clone_wakes_a_cancel_waiting_for_its_end() {
        let j = Arc::new(CloneJob {
            dest: PathBuf::new(),
            created: false,
            job: Job::new().unwrap(),
            cancelled: AtomicBool::new(false),
            status: Mutex::new(Status { state: "running", ..Default::default() }),
            ended: Condvar::new(),
        });
        let j2 = j.clone();
        let t = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            j2.finish(|s| s.state = "cancelled");
        });
        let at = Instant::now();
        let g = j.status.lock_recover();
        let _ = j.ended.wait_timeout_while(g, Duration::from_secs(30), |s| s.state == "running");
        assert!(at.elapsed() < Duration::from_secs(10));
        assert!(!j.running());
        t.join().unwrap();
    }

    use super::*;

    #[test]
    fn urls_are_checked_before_git_sees_them() {
        for ok in ["https://github.com/a/b.git", "ssh://git@host/a/b", "git@github.com:a/b.git", "  https://x.org/r  "] {
            assert!(check_url(ok, false).is_ok(), "{ok}");
        }
        for bad in [
            "",
            "-u",
            "--upload-pack=x",
            "ext::sh -c x",
            "http://x/y",
            "file:///tmp/r",
            "https://",
            "https:///x",
            "a b",
            "git@host:",
            "@h:p",
            "https://x/y\n--z",
        ] {
            assert!(check_url(bad, false).is_err(), "{bad:?}");
        }
        assert!(check_url("file:///tmp/r", true).is_ok());
        assert_eq!(msg::find(&check_url("-x", false).unwrap_err()).unwrap().code, "project.clone_url_invalid");
        assert_eq!(msg::find(&check_url(" ", false).unwrap_err()).unwrap().code, "project.clone_url_empty");
    }

    #[test]
    fn branches_cannot_be_options_or_refspecs() {
        assert!(check_branch("feature/x-1").is_ok());
        for bad in ["", "-b", "a..b", "a b", "a:b", "a~1"] {
            assert!(check_branch(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn folder_names_are_one_safe_segment() {
        for ok in ["app", "my-app.v2", "Çá"] {
            assert!(check_name(ok).is_ok(), "{ok}");
        }
        for bad in ["", ".", "..", "a/b", "a\\b", "C:x", "con", "NUL.txt", "com1", "lpt9", "x.", " x", "-x", "a*b", "a\tb"] {
            assert!(check_name(bad).is_err(), "{bad:?}");
        }
        assert!(check_name("com10").is_ok());
    }

    #[test]
    fn progress_lines_become_percent_and_phase() {
        assert_eq!(parse_progress("Receiving objects:  45% (45/100), 1.2 MiB | 2 MiB/s"), Some(("Receiving objects".into(), 45)));
        assert_eq!(parse_progress("remote: Counting objects: 100% (5/5), done."), Some(("Counting objects".into(), 100)));
        assert_eq!(parse_progress("Cloning into 'x'..."), None);
        assert_eq!(parse_progress("fatal: repository 'x' not found"), None);
        assert_eq!(overall("Receiving objects", 50), Some(40));
        assert_eq!(overall("Resolving deltas", 100), Some(100));
        assert_eq!(overall("Counting objects", 100), None);
    }

    #[test]
    fn clone_failures_are_told_apart() {
        let c = |t: &str| msg::find(&clone_error(&[t.to_string()], Some(128))).unwrap().code.clone();
        assert_eq!(c("fatal: Authentication failed for 'https://x/y'"), "project.clone_auth");
        assert_eq!(c("fatal: unable to access: Could not resolve host"), "project.clone_failed");
        assert_eq!(msg::find(&clone_error(&[], None)).unwrap().code, "project.clone_failed");
    }

    #[test]
    fn gh_repos_are_mapped_filtered_and_cut() {
        let j = r#"[{"nameWithOwner":"me/app","description":null,"isPrivate":true,"url":"u1","pushedAt":"t1"},
            {"nameWithOwner":"me/Tool","description":"A CLI","isPrivate":false,"url":"u2","pushedAt":"t2"}]"#;
        let all = parse_repos(j, "", 10).unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0], json!({"name_with_owner": "me/app", "description": "", "is_private": true, "url": "u1", "pushed_at": "t1"}));
        assert_eq!(parse_repos(j, "cli", 10).unwrap().len(), 1);
        assert_eq!(parse_repos(j, "", 1).unwrap().len(), 1);
        assert!(parse_repos("nope", "", 1).is_err());
    }

    #[test]
    fn scan_finds_repos_two_levels_down_and_skips_known_and_hidden() {
        let root = std::env::temp_dir().join(format!("axl-scan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for d in ["a/.git", "grp/b/.git", "grp/deep/c/.git", "node_modules/n/.git", ".hid/h/.git", "known/.git", "a/inner/.git"] {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        std::fs::write(root.join("a/.git/HEAD"), "ref: refs/heads/dev\n").unwrap();
        let known: HashSet<String> = [paths::norm_path(&root.join("known"))].into();
        let mut got = scan(std::slice::from_ref(&root), &known, Instant::now() + Duration::from_secs(5), 50);
        got.sort_by(|a, b| a.name.cmp(&b.name));
        let names: Vec<_> = got.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["a", "b"]);
        assert_eq!(got[0].branch.as_deref(), Some("dev"));
        assert_eq!(scan(std::slice::from_ref(&root), &known, Instant::now() + Duration::from_secs(5), 1).len(), 1);
        let _ = std::fs::remove_dir_all(&root);
    }
}
