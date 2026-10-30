#![cfg_attr(all(not(debug_assertions), windows), windows_subsystem = "windows")]

//! Thin UI host: every call is forwarded to the axelstra daemon over its pipe.
//! The daemon owns terminals and orchestration, so closing this window loses nothing.

use anyhow::{Context, Result};
use axelstra_core::sync::LockRecover;
use axelstra_daemon::client::{Client, Frame};
use axelstra_daemon::proto;
use serde_json::{json, Value};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::Manager;

struct State {
    /// request/response calls
    control: Mutex<Option<Client>>,
    /// keystrokes only, so typing never waits behind a slow control call
    input: Mutex<Option<Client>>,
    /// slow background polling (git status of every worktree, gh, transcript scans): its own connection,
    /// so creating a worktree or listing sessions never waits behind it
    bg: Mutex<Option<Client>>,
    /// the page's `call`s to the local daemon: the daemon runs one connection's calls in order, so these are several
    /// connections; the first two never take a slow method (git scans, gh, docker), so `project.list`/`session.list`
    /// always find a free one even while worktree scans are running
    lanes: axelstra_daemon::client::Lanes<Client>,
    /// the page's `callBg` polling: slow by nature, never on the quick lanes
    bg_lanes: axelstra_daemon::client::Lanes<Client>,
    /// per session id, one stop flag per attach token (a remounted view can attach again before the old one detaches)
    attachments: Mutex<std::collections::HashMap<u32, std::collections::HashMap<u64, Arc<AtomicBool>>>>,
    /// write side of the change-event connection (see `watch_start`)
    watch: Mutex<Option<Client>>,
    /// number of the newest `watch_start`; an older one that finishes later must not replace the newer connection
    watch_gen: std::sync::atomic::AtomicU64,
    /// an update already downloaded and verified, waiting for a restart (or for the app to be closed)
    pending: Mutex<Option<(tauri_plugin_updater::Update, Vec<u8>)>>,
}

impl Default for State {
    fn default() -> Self {
        State {
            control: Mutex::default(),
            input: Mutex::default(),
            bg: Mutex::default(),
            lanes: axelstra_daemon::client::Lanes::new(5, 2),
            bg_lanes: axelstra_daemon::client::Lanes::new(2, 0),
            attachments: Mutex::default(),
            watch: Mutex::default(),
            watch_gen: Default::default(),
            pending: Mutex::default(),
        }
    }
}

#[cfg(all(windows, target_env = "msvc", debug_assertions))]
mod debugcrt;
mod gate;
mod gpu;
mod hosts;
mod localfs;
mod logging;
mod native;
mod stale;
mod stats;
#[cfg(not(target_os = "linux"))]
mod voice;
// Linux has no sherpa-onnx static libs upstream: same commands, they only say dictation is unavailable.
#[cfg(target_os = "linux")]
#[path = "voice_linux.rs"]
mod voice;

/// A coded error of this host (`app.*`, see `axelstra_core::msg`): English text for logs, code and params for the UI.
macro_rules! host_err {
    ($($t:tt)*) => {
        CallError::from(&axelstra_core::coded!($($t)*))
    };
}

#[tauri::command]
async fn daemon_call_bg(state: tauri::State<'_, Arc<State>>, method: String, params: Value) -> Result<Value, CallError> {
    check_call(&method, &params)?;
    let st = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || routed(&st, &method, params, true)).await.map_err(|e| e.to_string())?
}

fn connect() -> Result<Client> {
    let link = Link {
        pipe: proto::pipe_name(),
        token: proto::load_or_create_token()?,
        app: app_build(),
        app_exe: std::env::current_exe().ok(),
        daemon_exe: &|c: &Client| c.server_exe(),
        start: &|| start_daemon().map(Some),
        memo: &STALE,
    };
    let (c, fresh) = connect_via(&link)?;
    if let Some(stale::Decision::Offer(_)) = fresh {
        use tauri::Emitter;
        if let Some(app) = APP.get() {
            let _ = app.emit("daemon-stale", ());
        }
    }
    Ok(c)
}

/// This app's build, as `stale::decide` compares it with the daemon's.
fn app_build() -> stale::Build {
    stale::Build { version: env!("CARGO_PKG_VERSION").into(), commit: env!("AXELSTRA_GIT_SHA").into() }
}

/// The daemon's build, from its `hello` (empty when it did not say).
fn daemon_build(c: &Client) -> stale::Build {
    stale::Build { version: c.version.clone(), commit: c.commit.clone() }
}

/// The last stale-daemon decision: the daemon run (`instance`) it was made for and what was decided. Every connection
/// (the lanes, the input and bg slots, each attached tab) goes through `connect`, so this lock makes one of them decide
/// per daemon run and the others reuse it: nobody replaces twice, nobody logs the same decision again.
static STALE: Mutex<Option<(u64, stale::Decision)>> = Mutex::new(None);

/// What `connect_via` needs, injectable so a test drives it against a fake daemon on a temp pipe.
struct Link<'a> {
    pipe: String,
    token: String,
    app: stale::Build,
    app_exe: Option<std::path::PathBuf>,
    /// where the daemon behind a connection runs from (`Client::server_exe`)
    daemon_exe: &'a dyn Fn(&Client) -> Option<std::path::PathBuf>,
    /// starts the app's own daemon; `None` when the test serves the pipe itself (no process to watch)
    start: &'a dyn Fn() -> Result<Option<std::process::Child>>,
    memo: &'a Mutex<Option<(u64, stale::Decision)>>,
}

/// Connects to the daemon on `link.pipe`, replacing one that is too old (`PROTO`, `SECURITY_MIN`) or stale (another
/// commit, see `stale::decide`) and starting one when none answers. Returns the decision when this call made it (a
/// daemon run seen for the first time), `None` when it was already made.
fn connect_via(link: &Link) -> Result<(Client, Option<stale::Decision>)> {
    let (pipe, token) = (link.pipe.as_str(), link.token.as_str());
    let mut memo = link.memo.lock_recover();
    if let Ok(mut c) = Client::connect(pipe, token) {
        let daemon = daemon_build(&c);
        if c.proto >= proto::PROTO && c.security >= proto::SECURITY_MIN {
            match *memo {
                Some((instance, d)) if instance == c.instance && d != stale::Decision::Replace => return Ok((c, None)),
                _ => {}
            }
            let (d, why) = judge(link, &mut c, &daemon);
            log_decision(d, &why, &link.app, &daemon);
            if d != stale::Decision::Replace {
                *memo = Some((c.instance, d));
                return Ok((c, Some(d)));
            }
        } else {
            // A daemon from before an update keeps running (so terminals and agents survive it). One that is too old
            // for this app, or older than the oldest one with the current security fixes (it reports no `security` at
            // all before they existed), is replaced; its terminals come back through session restore.
            let why = format!("too old (proto {}, security {})", c.proto, c.security);
            log_decision(stale::Decision::Replace, &why, &link.app, &daemon);
        }
        *memo = Some((c.instance, stale::Decision::Replace));
        wait_daemon_gone(c, pipe, token, None);
    }
    let mut child = (link.start)()?;
    // no event announces a named pipe (or socket) appearing: bounded backoff between looks, for as long as the launched
    // process lives (its death is the event that ends the wait). Only a process that lives and never opens the pipe
    // reaches the safety ceiling.
    let mut nap = Duration::from_millis(10);
    let deadline = Instant::now() + axelstra_daemon::client::ceiling();
    while Instant::now() < deadline {
        if let Ok(c) = Client::connect(pipe, token) {
            // the daemon just started is the app's own: not judged again, so an outdated exe beside the app cannot make
            // every later connection replace it once more
            *memo = Some((c.instance, stale::Decision::Keep));
            return Ok((c, None));
        }
        // the launched process ended: it handed over to a daemon that already runs or failed; one last look, no more waiting
        if child.as_mut().is_some_and(|ch| ch.try_wait().ok().flatten().is_some()) {
            let c = Client::connect(pipe, token).map_err(|_| axelstra_core::coded!("daemon.start_failed", "the daemon did not start"))?;
            *memo = Some((c.instance, stale::Decision::Keep));
            return Ok((c, None));
        }
        std::thread::sleep(nap);
        nap = (nap * 2).min(Duration::from_millis(200));
    }
    Err(axelstra_core::coded!("daemon.start_failed", "the daemon did not start"))
}

/// The stale-daemon decision for a daemon that is new enough, and why (for the log line). Where the daemon runs from
/// and what its sessions do are looked up only when the commits differ.
fn judge(link: &Link, c: &mut Client, daemon: &stale::Build) -> (stale::Decision, String) {
    use stale::{decide, Busy, Decision, Place, Reason};
    if decide(&link.app, daemon, Place::Beside, Busy::Idle) == Decision::Keep {
        let why = if link.app.known_commit().is_some() && daemon.known_commit().is_some() { "same commit" } else { "commit unknown" };
        return (Decision::Keep, why.into());
    }
    let exe = (link.daemon_exe)(c);
    let place = stale::place(link.app_exe.as_deref(), exe.as_deref());
    if place != Place::Beside {
        let at = exe.as_ref().map_or_else(|| "unknown".to_string(), |e| e.display().to_string());
        return (decide(&link.app, daemon, place, Busy::Idle), format!("stale, daemon not beside the app (exe {at})"));
    }
    // a failed list counts as busy: replacing must never cost work
    let busy = c.call("session.list", json!({})).map(|rows| stale::busy_of(&rows)).unwrap_or(Busy::Unknown);
    let d = decide(&link.app, daemon, place, busy);
    let why = match (d, busy) {
        (Decision::Offer(Reason::Busy), Busy::Unknown) => "stale, session list unavailable",
        (Decision::Offer(Reason::Busy), _) => "stale, sessions are busy",
        _ => "stale, idle, beside the app",
    };
    (d, why.into())
}

/// The one line that says what the app did with the daemon it found, and why.
fn log_decision(d: stale::Decision, why: &str, app: &stale::Build, daemon: &stale::Build) {
    let what = match d {
        stale::Decision::Keep => "kept",
        stale::Decision::Replace => "replaced",
        stale::Decision::Offer(_) => "kept, restart offered",
    };
    eprintln!("axelstra: daemon {what}: {why}; app {}, daemon {}", app.label(), daemon.label());
}

/// After `daemon.shutdown`, waits for the daemon to be gone. The event is the daemon closing our connection when its
/// process ends (a thread blocks on the read). `limit` is `None` when the app replaces an outdated daemon (it waits for
/// that event, with the call ceiling as the safety net); `Some` only when the app is quitting and must not hang on a
/// daemon that ignores the shutdown. If something still answers on the pipe afterwards (another daemon took over), a
/// short bounded backoff looks again (the pipe has no readiness event).
fn wait_daemon_gone(mut c: Client, pipe: &str, token: &str, limit: Option<Duration>) {
    let limit = limit.unwrap_or_else(axelstra_daemon::client::ceiling);
    let _ = c.call("daemon.shutdown", json!({}));
    let (tx, rx) = std::sync::mpsc::channel::<()>();
    std::thread::spawn(move || {
        while c.read().is_ok() {}
        let _ = tx.send(());
    });
    let _ = rx.recv_timeout(limit);
    let end = Instant::now() + Duration::from_secs(4);
    let mut nap = Duration::from_millis(10);
    while Client::connect(pipe, token).is_ok() && Instant::now() < end {
        std::thread::sleep(nap);
        nap = (nap * 2).min(Duration::from_millis(200));
    }
}

/// Starts `axelstra daemon` detached, so it outlives this window.
fn start_daemon() -> Result<std::process::Child> {
    let name = if cfg!(windows) { "axelstra.exe" } else { "axelstra" };
    let beside = std::env::current_exe()?.with_file_name(name);
    // never `axelstra` from PATH in a release: whatever sits first on PATH would run as the daemon, holding the token
    let exe = if beside.exists() || !cfg!(debug_assertions) { beside } else { "axelstra".into() };
    let mut cmd = Command::new(exe);
    cmd.arg("daemon").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        cmd.creation_flags(DETACHED_PROCESS | axelstra_core::proc::NO_WINDOW);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // own process group: closing the app (or its terminal) does not signal the daemon
        cmd.process_group(0);
    }
    cmd.spawn().context("start axelstra daemon (build it with `cargo build -p axelstra`)")
}

static APP: std::sync::OnceLock<tauri::AppHandle> = std::sync::OnceLock::new();
static LAST_INSTANCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// `connect`, and tells the UI when the daemon behind it is a new run (it crashed or was replaced):
/// its sessions are gone and the UI restores them from the saved list.
fn reconnect() -> Result<Client> {
    use tauri::Emitter;
    // a restart can take seconds: the UI shows a state instead of looking frozen
    if let Some(app) = APP.get() {
        let _ = app.emit("daemon-reconnecting", ());
    }
    let c = connect();
    if let Some(app) = APP.get() {
        let _ = app.emit("daemon-reconnected", json!({"ok": c.is_ok()}));
    }
    let c = c?;
    let prev = LAST_INSTANCE.swap(c.instance, Ordering::SeqCst);
    if prev != 0 && c.instance != prev {
        if let Some(app) = APP.get() {
            let _ = app.emit("daemon-restarted", ());
        }
    }
    Ok(c)
}

/// One key of the daemon's `settings.json` (null when missing): the app reads it directly, like the daemon does.
fn setting(key: &str) -> Value {
    std::fs::read_to_string(proto::data_dir().join("settings.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        // the file is the envelope {"v":N,"data":{..}}; a file from an older daemon is the bare object
        .map(|v| if v["v"].is_u64() && v["data"].is_object() { v["data"][key].clone() } else { v[key].clone() })
        .unwrap_or(Value::Null)
}

/// Calls that only read: safe to run twice if the connection broke after the daemon may have received them.
fn read_only(method: &str) -> bool {
    let tail = method.rsplit('.').next().unwrap_or(method);
    matches!(
        tail,
        "list"
            | "status"
            | "get"
            | "read"
            | "screen"
            | "usage"
            | "ports"
            | "pending"
            | "info"
            | "stats"
            | "log"
            | "inbox"
            | "thread"
            | "search"
            | "logo"
            | "dir"
            | "limits"
            | "diff"
            | "changes"
            | "suggest"
    )
}

/// Daemon methods the webview may forward through `daemon_call` / `daemon_call_bg`: exactly the ones the frontend
/// calls (a unit test reads `app/src` and fails when this list and the sources drift apart). Anything else is refused,
/// so a compromised page cannot reach what the UI never uses (`session.pending_clear`, `msg.send`, `flow.set`...).
const ALLOWED_METHODS: &[&str] = &[
    "agent.search",
    "agent.status",
    "agent.usage",
    "agent.worktree_usage",
    "app.dir",
    "automation.delete",
    "automation.list",
    "automation.run",
    "automation.save",
    "codex.limits",
    "computer.enable",
    "computer.status",
    "daemon.shutdown",
    "backup.export",
    "env.tools",
    "triggers.events",
    "triggers.get",
    "triggers.preview",
    "triggers.set",
    "notices.take",
    "dispatch.meta",
    "flow.list",
    "files.path_action",
    "files.resolve_path",
    "fs.create",
    "fs.delete",
    "fs.grep",
    "fs.list",
    "fs.read",
    "fs.rename",
    "fs.search",
    "fs.stat",
    "fs.write",
    "gh.comments",
    "gh.pr_refresh",
    "gh.list",
    "gh.repos",
    "gh.send_comment",
    "git.changes",
    "git.commit",
    "git.diff",
    "git.log",
    "git.pr",
    "git.push",
    "git.show",
    "git.suggest",
    "git.suggest_pr",
    "docker.action",
    "docker.containers",
    "docker.fs_list",
    "docker.fs_read",
    "docker.images",
    "docker.inspect",
    "docker.logs",
    "docker.networks",
    "docker.open_logs",
    "docker.open_shell",
    "docker.stats",
    "docker.status",
    "docker.volumes",
    "agents.list",
    "git.checks",
    "git.timeline",
    "git.merge",
    "project.add_discovered",
    "project.clone",
    "project.clone_cancel",
    "project.clone_status",
    "project.discover",
    "project.env_preview",
    "project.init",
    "worktree.rename",
    "dispatch.overlaps",
    "dispatch.overlap_files",
    "worktree.rebase",
    "notes.read",
    "notes.export",
    "worktree.overview",
    "worktree.sweep",
    "disk.status",
    "worktree.integrate",
    "integrate.undo",
    "orch.cancel",
    "project.integrate",
    "janitor.events",
    "budget.status",
    "budget.check",
    "budget.events",
    "notes.read_file",
    "notes.worktree",
    "worktree.set_parent",
    "hooks.install",
    "hooks.status",
    "host.browse",
    "host.info",
    "kanban.delete",
    "kanban.list",
    "kanban.move",
    "kanban.save",
    "open.editor",
    "orch.limits",
    "orch.trust_info",
    "orch.retry",
    "orch.start",
    "orch.status",
    "ping",
    "pm.detect",
    "ports.list",
    "project.list",
    "project.logo",
    "project.remove",
    "project.remove_preview",
    "project.setup",
    "session.hibernate",
    "session.kill",
    "session.list",
    "session.pending",
    "session.pending_remove",
    "session.remove",
    "session.rename",
    "session.resize",
    "session.resources",
    "mem.status",
    "mem.config",
    "mem.pause",
    "mem.resume",
    "session.screen",
    "session.spawn",
    "session.wake",
    "settings.get",
    "settings.set",
    "worktree.create",
    "worktree.from_pr",
    "worktree.remove",
    "worktree.status",
];

fn check_allowed(method: &str) -> Result<(), CallError> {
    if ALLOWED_METHODS.contains(&method) {
        Ok(())
    } else {
        Err(host_err!("app.method_not_allowed", "daemon method not allowed: {method}", method = method))
    }
}

/// Programs a webview-requested terminal may start: shells only (agents run inside one). The name is compared
/// without directory, extension and case, so `C:\Program Files\PowerShell\7\pwsh.exe` and `/bin/zsh` pass.
const SHELLS: &[&str] =
    &["pwsh", "powershell", "cmd", "bash", "zsh", "sh", "dash", "fish", "ksh", "tcsh", "csh", "nu", "wsl", "xonsh", "elvish"];

fn is_shell(program: &str) -> bool {
    let name = program.rsplit(['/', '\\']).next().unwrap_or(program).to_ascii_lowercase();
    let stem = name.strip_suffix(".exe").unwrap_or(&name);
    SHELLS.contains(&stem)
}

/// Arguments of the calls that can run code or write files, beyond the method allow-list: `session.spawn` starts a
/// shell only (and sets no environment but `AXELSTRA_*`), `fs.write` takes a relative path outside `.git`.
fn check_params(method: &str, params: &Value) -> Result<(), CallError> {
    match method {
        "session.spawn" => {
            let program = params["program"].as_str().unwrap_or_default();
            if !is_shell(program) {
                return Err(host_err!("app.program_not_allowed", "program not allowed: {program}", program = program));
            }
            if let Some(env) = params["env"].as_object() {
                if let Some(k) = env.keys().find(|k| !k.starts_with("AXELSTRA_")) {
                    return Err(host_err!("app.env_not_allowed", "environment variable not allowed: {name}", name = k));
                }
            }
            Ok(())
        }
        // the host registry changes only through `host_add` / `host_remove`, which confirm in a native dialog
        "settings.set" if params.get("hosts").is_some() => {
            Err(host_err!("app.hosts_own_commands", "hosts can only be changed through their own commands"))
        }
        // the UI never overrides the agent command: a page that did could run any program unsupervised
        "orch.start" if !params["agent"].is_null() => Err(host_err!("app.agent_override", "overriding the agent command is not allowed")),
        "fs.write" => {
            let path = params["path"].as_str().unwrap_or_default();
            let p = std::path::Path::new(path);
            let plain = !path.is_empty()
                && !p.is_absolute()
                && p.components().all(|c| matches!(c, std::path::Component::Normal(_)))
                && !path.split(['/', '\\']).any(|c| c.trim_end_matches(['.', ' ']).eq_ignore_ascii_case(".git"));
            if plain {
                Ok(())
            } else {
                Err(host_err!("app.path_not_allowed", "path not allowed: {path}", path = path))
            }
        }
        _ => Ok(()),
    }
}

fn check_call(method: &str, params: &Value) -> Result<(), CallError> {
    check_allowed(method)?;
    check_params(method, params)
}

/// Only one caller restarts the daemon connection at a time; the others wait for it outside any slot lock.
static RECONNECT: gate::Gate = gate::Gate::new();
/// Callers wait for another one's reconnect until it ends (the gate wakes them); this only bounds a leader that is lost.
fn reconnect_wait() -> Duration {
    axelstra_daemon::client::ceiling()
}

/// Fills an empty `slot` with a live connection. The slot lock is never held while connecting or sleeping, so calls on
/// other slots (and `slot` itself) are not stuck behind a restart that can take seconds.
fn reconnect_into(slot: &Mutex<Option<Client>>) -> Result<()> {
    let deadline = std::time::Instant::now() + reconnect_wait();
    loop {
        if slot.lock_recover().is_some() {
            return Ok(());
        }
        // the one that reconnects leads; the others sleep until it ends (no timer) or `deadline` passes
        match RECONNECT.enter(deadline) {
            gate::Entry::Lead(_lead) => {
                // another caller may have finished while this one took the gate
                if slot.lock_recover().is_some() {
                    return Ok(());
                }
                let c = reconnect()?;
                *slot.lock_recover() = Some(c);
                return Ok(());
            }
            gate::Entry::Waited => {}
            gate::Entry::TimedOut => {
                return Err(axelstra_core::coded!("app.daemon_restarting", "the daemon is restarting; try again in a moment"));
            }
        }
    }
}

fn with_client<T>(slot: &Mutex<Option<Client>>, retry: bool, f: impl Fn(&mut Client) -> Result<T>) -> Result<T> {
    let mut g = slot.lock_recover();
    if g.is_none() {
        drop(g);
        reconnect_into(slot)?;
        g = slot.lock_recover();
    }
    let Some(c) = g.as_mut() else { return Err(axelstra_core::coded!("app.no_daemon", "no connection to the daemon")) };
    match f(c) {
        Ok(v) => Ok(v),
        // the daemon answered with an error: the connection is fine and the call must not run twice
        Err(e) if e.downcast_ref::<axelstra_daemon::client::Remote>().is_some() => Err(e),
        Err(e) => {
            // the connection may be stale (daemon restarted): reconnect. Run the call again only when that
            // cannot repeat a side effect (it never left, or it only reads)
            let never_sent = e.downcast_ref::<axelstra_daemon::client::NotSent>().is_some();
            *g = None;
            drop(g);
            reconnect_into(slot)?;
            if retry || never_sent {
                match slot.lock_recover().as_mut() {
                    Some(c) => f(c),
                    None => Err(e),
                }
            } else {
                Err(e.context(axelstra_core::coded!(
                    "app.daemon_dropped",
                    "the connection to the daemon dropped during the request; check the state before repeating it"
                )))
            }
        }
    }
}

#[tauri::command]
async fn daemon_call(state: tauri::State<'_, Arc<State>>, method: String, params: Value) -> Result<Value, CallError> {
    check_call(&method, &params)?;
    let st = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || routed(&st, &method, params, false)).await.map_err(|e| e.to_string())?
}

/// One call, to this machine's daemon or (when its paths or session id say so) to a remote host's.
fn routed(st: &State, method: &str, params: Value, bg: bool) -> Result<Value, CallError> {
    // one slot per call: a slow reply only holds its own connection (see `Lanes`)
    let local = |m: &str, p: Value| {
        let claim = if bg { st.bg_lanes.pick(true) } else { st.lanes.pick(axelstra_daemon::client::is_slow(m)) };
        with_client(claim.slot(), read_only(m), |c| c.call(m, p.clone()))
    };
    hosts::route(method, params, bg, read_only(method), &local).map_err(|e| CallError::from(&e))
}

/// What a failed call rejects with in the webview: the English `message` (what an old frontend shows) and, when the
/// daemon (or this host) raised a coded error, its `code` and `params`, which the UI translates. See `axelstra_core::msg`.
#[derive(serde::Serialize, Debug)]
struct CallError {
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    params: Option<std::collections::BTreeMap<String, String>>,
}

impl From<&anyhow::Error> for CallError {
    fn from(e: &anyhow::Error) -> Self {
        let c = axelstra_core::msg::find(e);
        CallError { message: format!("{e:#}"), code: c.map(|c| c.code.clone()), params: c.map(|c| c.params.clone()) }
    }
}

impl From<&str> for CallError {
    fn from(message: &str) -> Self {
        CallError::from(message.to_string())
    }
}

impl From<String> for CallError {
    fn from(message: String) -> Self {
        CallError { message, code: None, params: None }
    }
}

/// Asks for a folder in the native dialog. The path never passes through the webview: a compromised page could
/// otherwise name any folder for `project.add` or `backup.restore`. `None` = cancelled.
fn pick_folder(app: &tauri::AppHandle, title: &str, start: Option<std::path::PathBuf>) -> Result<Option<String>, CallError> {
    use tauri_plugin_dialog::DialogExt;
    let mut d = app.dialog().file().set_title(title);
    if let Some(s) = start.filter(|s| s.is_dir()) {
        d = d.set_directory(s);
    }
    match d.blocking_pick_folder() {
        None => Ok(None),
        Some(f) => f.into_path().map(|p| Some(p.to_string_lossy().into_owned())).map_err(|e| CallError::from(e.to_string())),
    }
}

/// Native folder pick; the path goes back to the page only as the parent or destination of `project.init` and
/// `project.clone` (the daemon validates both). Returns null when cancelled.
#[tauri::command]
async fn folder_pick(app: tauri::AppHandle, title: String) -> Result<Option<String>, CallError> {
    tauri::async_runtime::spawn_blocking(move || pick_folder(&app, &title, None)).await.map_err(|e| e.to_string())?
}

/// Native folder pick, then `project.add` with that path. Returns the daemon's project, or null when cancelled.
#[tauri::command]
async fn project_add_pick(app: tauri::AppHandle, state: tauri::State<'_, Arc<State>>, title: String) -> Result<Value, CallError> {
    let st = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let Some(path) = pick_folder(&app, &title, None)? else { return Ok(Value::Null) };
        with_client(&st.control, false, |c| c.call("project.add", json!({"path": path}))).map_err(|e| CallError::from(&e))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Native pick of the backup folder (it opens on `<data dir>/backups`), then `backup.restore` with that path.
/// Returns the daemon's answer, or null when cancelled.
#[tauri::command]
async fn backup_restore_pick(app: tauri::AppHandle, state: tauri::State<'_, Arc<State>>, title: String) -> Result<Value, CallError> {
    let st = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let Some(path) = pick_folder(&app, &title, Some(proto::data_dir().join("backups")))? else {
            return Ok(Value::Null);
        };
        with_client(&st.control, false, |c| c.call("backup.restore", json!({"path": path}))).map_err(|e| CallError::from(&e))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// The workspace file picked for an import and the folder map chosen so far. They live here and not in the webview: the
/// file and the folders come from the native dialogs, so a compromised page cannot name a file to read or a folder to
/// register as a project (S4b); the page only chooses mode and conflict handling.
type WorkspacePick = (Option<String>, Vec<(String, String)>);
static WORKSPACE_PICK: Mutex<WorkspacePick> = Mutex::new((None, Vec::new()));

/// `workspace.import_preview` of the picked file with the folder map so far, plus the map and the file name for the page.
fn workspace_preview(st: &State) -> Result<Value, CallError> {
    let (file, map) = {
        let g = WORKSPACE_PICK.lock().unwrap_or_else(|e| e.into_inner());
        (g.0.clone(), g.1.clone())
    };
    let Some(file) = file else { return Err(host_err!("app.no_file", "no file was chosen")) };
    let pairs: Vec<Value> = map.iter().map(|(o, n)| json!([o, n])).collect();
    let mut v = with_client(&st.control, false, |c| c.call("workspace.import_preview", json!({"path": file, "map": pairs})))
        .map_err(|e| CallError::from(&e))?;
    v["map"] = Value::Array(pairs);
    v["file"] = json!(std::path::Path::new(&file).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
    Ok(v)
}

/// Native "save as" dialog, then `workspace.export` to that file. Returns the daemon's answer, or null when cancelled.
#[tauri::command]
async fn workspace_export_pick(app: tauri::AppHandle, state: tauri::State<'_, Arc<State>>, title: String) -> Result<Value, CallError> {
    use tauri_plugin_dialog::DialogExt;
    let st = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let picked = app
            .dialog()
            .file()
            .set_title(&title)
            .set_file_name("axelstra-workspace.json")
            .add_filter("Axelstra", &["json"])
            .blocking_save_file();
        let Some(path) = picked else { return Ok(Value::Null) };
        let path = path.into_path().map_err(|e| e.to_string())?.to_string_lossy().into_owned();
        with_client(&st.control, false, |c| c.call("workspace.export", json!({"path": path}))).map_err(|e| CallError::from(&e))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Native "open" dialog for a workspace file; remembers it and returns its preview (null when cancelled).
#[tauri::command]
async fn workspace_import_pick(app: tauri::AppHandle, state: tauri::State<'_, Arc<State>>, title: String) -> Result<Value, CallError> {
    use tauri_plugin_dialog::DialogExt;
    let st = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let Some(path) = app.dialog().file().set_title(&title).add_filter("Axelstra", &["json"]).blocking_pick_file() else {
            return Ok(Value::Null);
        };
        let path = path.into_path().map_err(|e| e.to_string())?.to_string_lossy().into_owned();
        *WORKSPACE_PICK.lock().unwrap_or_else(|e| e.into_inner()) = (Some(path), Vec::new());
        workspace_preview(&st).inspect_err(|_| *WORKSPACE_PICK.lock().unwrap_or_else(|e| e.into_inner()) = (None, Vec::new()))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Native folder pick for the new place of the projects under `old` (a path from the file), then the preview again.
#[tauri::command]
async fn workspace_map_pick(
    app: tauri::AppHandle,
    state: tauri::State<'_, Arc<State>>,
    title: String,
    old: String,
) -> Result<Value, CallError> {
    let st = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        if old.is_empty() || old.len() > 1024 || WORKSPACE_PICK.lock().unwrap_or_else(|e| e.into_inner()).0.is_none() {
            return Err(host_err!("app.bad_request", "invalid request"));
        }
        let Some(new) = pick_folder(&app, &title, None)? else { return Ok(Value::Null) };
        {
            let mut g = WORKSPACE_PICK.lock().unwrap_or_else(|e| e.into_inner());
            g.1.retain(|(o, _)| *o != old);
            if g.1.len() >= 32 {
                return Err(host_err!("app.too_many_mappings", "too many folder mappings"));
            }
            g.1.push((old, new));
        }
        workspace_preview(&st)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Forgets the folder map and previews the picked file again.
#[tauri::command]
async fn workspace_map_clear(state: tauri::State<'_, Arc<State>>) -> Result<Value, CallError> {
    let st = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        WORKSPACE_PICK.lock().unwrap_or_else(|e| e.into_inner()).1.clear();
        workspace_preview(&st)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// `workspace.import` of the picked file with the folder map; the page only says how (`mode`, `conflicts`).
#[tauri::command]
async fn workspace_import_apply(state: tauri::State<'_, Arc<State>>, mode: String, conflicts: String) -> Result<Value, CallError> {
    let st = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let (file, map) = {
            let g = WORKSPACE_PICK.lock().unwrap_or_else(|e| e.into_inner());
            (g.0.clone(), g.1.clone())
        };
        let Some(file) = file else { return Err(host_err!("app.no_file", "no file was chosen")) };
        let pairs: Vec<Value> = map.iter().map(|(o, n)| json!([o, n])).collect();
        let r = with_client(&st.control, false, |c| {
            c.call("workspace.import", json!({"path": file, "mode": mode, "conflicts": conflicts, "map": pairs}))
        })
        .map_err(|e| CallError::from(&e));
        if r.is_ok() {
            *WORKSPACE_PICK.lock().unwrap_or_else(|e| e.into_inner()) = (None, Vec::new());
        }
        r
    })
    .await
    .map_err(|e| e.to_string())?
}

/// The page closed the import dialog: forget the picked file.
#[tauri::command]
fn workspace_import_cancel() {
    *WORKSPACE_PICK.lock().unwrap_or_else(|e| e.into_inner()) = (None, Vec::new());
}

/// A native yes/no the page cannot answer for the user (the registry and remote projects are the user's to decide).
fn confirm_native(app: &tauri::AppHandle, c: native::Confirm) -> bool {
    use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
    app.dialog()
        .message(c.text)
        .title(c.title)
        .kind(MessageDialogKind::Warning)
        .buttons(MessageDialogButtons::OkCancelCustom(c.ok.to_string(), c.cancel.to_string()))
        .blocking_show()
}

/// The language the UI is set to, for the texts the host shows by itself (pt-PT while the daemon cannot say).
fn ui_lang(st: &State) -> native::Lang {
    with_client(&st.control, true, |c| c.call("settings.get", json!({})))
        .map(|s| native::lang_from_settings(&s))
        .unwrap_or(native::Lang::Pt)
}

/// The registered remote hosts and how each connection stands.
#[tauri::command]
fn hosts_list() -> Vec<Value> {
    hosts::list()
}

/// Registers a host after the user confirmed it in a native dialog (a page cannot make the app run `ssh` somewhere).
#[tauri::command]
async fn host_add(
    app: tauri::AppHandle,
    state: tauri::State<'_, Arc<State>>,
    name: String,
    spec: String,
    bin: Option<String>,
) -> Result<Value, CallError> {
    let st = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        axelstra_daemon::remote::RemoteSpec::parse(&spec).map_err(|e| CallError::from(&e))?;
        if !confirm_native(&app, native::add_host(ui_lang(&st), name.trim(), &spec)) {
            return Err(host_err!("app.cancelled", "cancelled"));
        }
        let h = with_client(&st.control, false, |c| hosts::add(c, &name, &spec, bin.as_deref())).map_err(|e| CallError::from(&e))?;
        Ok(h)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Forgets a host and drops its connections (what runs there keeps running).
#[tauri::command]
async fn host_remove(app: tauri::AppHandle, state: tauri::State<'_, Arc<State>>, id: String) -> Result<(), CallError> {
    let st = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        if !confirm_native(&app, native::remove_host(ui_lang(&st), &id)) {
            return Err(host_err!("app.cancelled", "cancelled"));
        }
        with_client(&st.control, false, |c| hosts::remove(c, &id)).map_err(|e| CallError::from(&e))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Tries the host again now (the keeper thread also does it by itself, after a wait).
#[tauri::command]
async fn host_reconnect(id: String) -> Result<Value, CallError> {
    tauri::async_runtime::spawn_blocking(move || {
        let h = hosts::get(&id).ok_or_else(|| host_err!("app.host_unknown", "unknown host: {host}", host = id))?;
        let _ = h.connect_now();
        Ok(h.describe())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Adds a folder of a remote host as a project, after the user confirmed it in a native dialog.
#[tauri::command]
async fn project_add_remote(
    app: tauri::AppHandle,
    state: tauri::State<'_, Arc<State>>,
    host: String,
    path: String,
) -> Result<Value, CallError> {
    let st = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let h = hosts::get(&host).ok_or_else(|| host_err!("app.host_unknown", "unknown host: {host}", host = host))?;
        let t = path.trim();
        let absolute = t.starts_with('/') || t.as_bytes().get(1) == Some(&b':');
        if t.is_empty() || t.len() > 1024 || t.chars().any(char::is_control) || !absolute {
            return Err(host_err!("app.remote_path_invalid", "invalid remote path: it must be absolute"));
        }
        if !confirm_native(&app, native::add_remote_project(ui_lang(&st), t, &h.def.name)) {
            return Err(host_err!("app.cancelled", "cancelled"));
        }
        let mut v = hosts::with_remote(&h, hosts::Which::Control, false, |c| c.call("project.add", json!({"path": t})))
            .map_err(|e| CallError::from(&e))?;
        hosts::tag_project(&h.def.id, h.def.slot, &mut v);
        Ok(v)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Typing lag diagnosis: slow steps go to `<data dir>/lag.log` (shared with the daemon's own slow calls).
fn lag_note(what: &str, took: std::time::Duration) {
    axelstra_daemon::lag::note(what, took, 40);
}

/// The webview reports its own stalls (main thread blocked) here.
#[tauri::command]
fn lag_log(what: String, ms: u64) {
    lag_note(&format!("webview {what}"), std::time::Duration::from_millis(ms));
}

/// Keystrokes go to the daemon through one writer thread per host (slot 0 is this machine): in order (a lock race
/// between per-key tasks could swap them), a stalled daemon never blocks the UI, and a slow remote link never holds up
/// typing in a local terminal. Bytes queued for the same session are sent as one frame.
type InputTx = std::sync::mpsc::Sender<(u32, Vec<u8>)>;
static INPUT_TX: Mutex<Vec<(u32, InputTx)>> = Mutex::new(Vec::new());

fn input_sender(st: &Arc<State>, slot: u32) -> InputTx {
    let mut all = INPUT_TX.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, tx)) = all.iter().find(|(s, _)| *s == slot) {
        return tx.clone();
    }
    let (tx, rx) = std::sync::mpsc::channel::<(u32, Vec<u8>)>();
    let st = st.clone();
    std::thread::spawn(move || {
        while let Ok((mut id, mut data)) = rx.recv() {
            let mut pending: Vec<(u32, Vec<u8>)> = vec![];
            while let Ok((i, d)) = rx.try_recv() {
                if i == id {
                    data.extend(d);
                } else {
                    pending.push((id, std::mem::take(&mut data)));
                    id = i;
                    data = d;
                }
            }
            pending.push((id, data));
            for (id, data) in pending {
                let t = std::time::Instant::now();
                let (_, raw) = hosts::split_gid(id);
                let _ = match hosts::by_slot(slot) {
                    Some(h) if slot != 0 => hosts::with_remote(&h, hosts::Which::Input, true, |c| c.send_input(raw, &data)),
                    _ => with_client(&st.input, true, |c| c.send_input(id, &data)),
                };
                lag_note("keystroke to daemon", t.elapsed());
            }
        }
    });
    all.push((slot, tx.clone()));
    tx
}

#[tauri::command]
fn session_write(state: tauri::State<'_, Arc<State>>, id: u32, data: String) -> Result<(), CallError> {
    // keystrokes only go to a session this window has open (attached), not to any id a page can guess
    if !state.attachments.lock_recover().get(&id).is_some_and(|m| !m.is_empty()) {
        return Err(host_err!("app.session_not_attached", "session {id} is not attached", id = id));
    }
    input_sender(state.inner(), hosts::split_gid(id).0).send((id, data.into_bytes())).map_err(|e| CallError::from(e.to_string()))
}

/// Streams a session to the webview: raw bytes for output, a JSON string for events.
#[tauri::command]
async fn session_attach(state: tauri::State<'_, Arc<State>>, id: u32, on_data: Channel<InvokeResponseBody>) -> Result<u64, CallError> {
    static NEXT_TOKEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let token = NEXT_TOKEN.fetch_add(1, Ordering::SeqCst);
    let stop = Arc::new(AtomicBool::new(false));
    state.attachments.lock_recover().entry(id).or_default().insert(token, stop.clone());
    let st = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || -> Result<u64, CallError> {
        let forget = || {
            if let Some(m) = st.attachments.lock_recover().get_mut(&id) {
                m.remove(&token);
            }
        };
        let (slot, raw) = hosts::split_gid(id);
        let remote = if slot == 0 { None } else { hosts::by_slot(slot) };
        if slot != 0 && remote.is_none() {
            forget();
            return Err(host_err!("app.session_host_gone", "session {id} belongs to a host that no longer exists", id = id));
        }
        let mut c = match &remote {
            Some(h) => h.connect_stream(),
            None => connect(),
        }
        .map_err(|e| {
            forget();
            format!("{e:#}")
        })?;
        c.call("session.subscribe", json!({"id": raw})).map_err(|e| {
            forget();
            format!("{e:#}")
        })?;
        std::thread::spawn(move || {
            while let Ok(f) = c.read() {
                if stop.load(Ordering::SeqCst) {
                    break;
                }
                let sent = match f {
                    // the daemon's keep-alive (see the forwarder): only there so a detached tab can hang up
                    Frame::Output { data, .. } if data.is_empty() => Ok(()),
                    Frame::Output { data, .. } => on_data.send(InvokeResponseBody::Raw(data)),
                    Frame::Event(mut e) => {
                        if let Some(h) = &remote {
                            let name = e["event"].as_str().unwrap_or_default().to_string();
                            hosts::tag_result(&name, &h.def.id, h.def.slot, &mut e);
                        }
                        on_data.send(InvokeResponseBody::Json(e.to_string()))
                    }
                    Frame::Response(_) => Ok(()),
                };
                if sent.is_err() {
                    break;
                }
            }
        });
        Ok(token)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Starts the one change-event stream of the window: `fs.changed` events arrive as JSON strings.
/// The connection stays open; `watch_set` points it at a worktree.
#[tauri::command]
async fn watch_start(
    app: tauri::AppHandle,
    state: tauri::State<'_, Arc<State>>,
    on_event: Channel<InvokeResponseBody>,
) -> Result<(), CallError> {
    let st = state.inner().clone();
    // numbered in the order the page asked: the connections below finish in any order, and the page only listens to the
    // channel of its newest call (a stale connection installed last would receive events nobody reads)
    let gen = st.watch_gen.fetch_add(1, Ordering::SeqCst) + 1;
    tauri::async_runtime::spawn_blocking(move || -> Result<(), CallError> {
        let c = connect().map_err(|e| CallError::from(&e))?;
        let mut rd = c.try_clone().map_err(|e| CallError::from(&e))?;
        {
            let mut w = st.watch.lock_recover();
            if st.watch_gen.load(Ordering::SeqCst) != gen {
                return Ok(());
            }
            *w = Some(c);
        }
        // remote hosts push their events into the same channel
        hosts::set_events(on_event.clone());
        let st2 = st.clone();
        std::thread::spawn(move || {
            // `read_shared`: the page writes to this connection (watch_set) while this thread waits for events
            while let Ok(f) = rd.read_shared() {
                if let Frame::Event(e) = f {
                    if on_event.send(InvokeResponseBody::Json(e.to_string())).is_err() {
                        break;
                    }
                }
            }
            // the connection dropped (daemon gone) or the channel closed: the UI starts the stream again; a replaced
            // connection ending is expected and says nothing (it would make the page tear down the live one)
            if st2.watch_gen.load(Ordering::SeqCst) == gen {
                use tauri::Emitter;
                let _ = app.emit("watch-ended", ());
            }
        });
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Watch this worktree (replacing the previous one), or nothing when `worktree` is null.
#[tauri::command]
async fn watch_set(state: tauri::State<'_, Arc<State>>, worktree: Option<String>) -> Result<(), CallError> {
    let st = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || -> Result<(), CallError> {
        let mut g = st.watch.lock_recover();
        let c = g.as_mut().ok_or("watch not started")?;
        // a remote worktree is watched by that host's daemon; this machine's one then watches nothing
        match hosts::watch_target(worktree.as_deref()).map_err(|e| CallError::from(&e))? {
            Some(w) => c.send("watch.subscribe", json!({"worktree": w})),
            None => c.send("watch.unsubscribe", json!({})),
        }
        .map_err(|e| CallError::from(&e))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
/// Stops the attach `token` returned by `session_attach`; without a token, every attach of the session.
fn session_detach(state: tauri::State<'_, Arc<State>>, id: u32, token: Option<u64>) {
    let mut g = state.attachments.lock_recover();
    let Some(m) = g.get_mut(&id) else { return };
    match token {
        Some(t) => {
            if let Some(stop) = m.remove(&t) {
                stop.store(true, Ordering::SeqCst);
            }
        }
        None => m.drain().for_each(|(_, stop)| stop.store(true, Ordering::SeqCst)),
    }
    if m.is_empty() {
        g.remove(&id);
    }
}

/// What a "new terminal" should run on this OS: the user's login shell on Unix, PowerShell on Windows.
/// Desktop toast whose click brings the window forward and tells the UI which session it came from.
/// Returns false where the OS toast has no click callback (the UI then uses the plain notification plugin).
#[tauri::command]
///
/// Honors the settings `notify_mode` (all|waiting|off; `waiting` only notifies when the UI says the agent
/// is waiting on the user, via `waiting: true`) and `notify_sound` (default true).
fn notify_session(app: tauri::AppHandle, title: String, body: String, worktree: String, session: u32, waiting: Option<bool>) -> bool {
    match setting("notify_mode").as_str().unwrap_or("all") {
        "off" => return true,
        "waiting" if waiting != Some(true) => return true,
        _ => {}
    }
    let sound = setting("notify_sound") != json!(false);
    // the taskbar button flashes until the window is brought back, even when the toast was missed
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.request_user_attention(Some(tauri::UserAttentionType::Informational));
    }
    #[cfg(windows)]
    {
        use tauri::Emitter;
        // Windows hides toasts (and their sound) while a game or presentation is fullscreen; a plain system
        // beep still gets through, so the toast stays silent and this one is the audible part.
        if sound {
            unsafe {
                windows_sys::Win32::System::Diagnostics::Debug::MessageBeep(windows_sys::Win32::UI::WindowsAndMessaging::MB_ICONASTERISK)
            };
        }
        let handle = app.clone();
        let shown = tauri_winrt_notification::Toast::new(&app.config().identifier)
            .sound(None)
            .title(&title)
            .text1(&body)
            .on_activated(move |_| {
                if let Some(w) = handle.get_webview_window("main") {
                    let _ = w.unminimize();
                    let _ = w.show();
                    let _ = w.set_focus();
                }
                let _ = handle.emit("notification-click", json!({"worktree": worktree, "session": session}));
                Ok(())
            })
            .show();
        shown.is_ok()
    }
    #[cfg(not(windows))]
    {
        let _ = (app, title, body, worktree, session, sound);
        false
    }
}

/// The UI confirmed the close: exit for real (the exit hook still installs a downloaded update).
#[tauri::command]
async fn app_quit(app: tauri::AppHandle, keep: Option<bool>) {
    // by default nothing is left running: the daemon ends its terminals and agents and exits too. They come
    // back through session restore the next time the app opens. `keep` leaves them (and automations) running.
    if keep != Some(true) {
        // waits up to 3s for the pipe to close: off the main thread
        let _ = tauri::async_runtime::spawn_blocking(|| {
            if let Ok(token) = proto::load_or_create_token() {
                let pipe = proto::pipe_name();
                if let Ok(c) = Client::connect(&pipe, &token) {
                    wait_daemon_gone(c, &pipe, &token, Some(Duration::from_secs(3)));
                }
            }
        })
        .await;
    }
    app.exit(0);
}

/// Which of the tools the app leans on are not on PATH (git, claude, codex, gh) plus other agent CLIs worth offering (gemini).
#[tauri::command]
fn missing_tools() -> Vec<String> {
    ["git", "claude", "codex", "gh", "gemini"].into_iter().filter(|t| !on_path(t)).map(String::from).collect()
}

fn on_path(name: &str) -> bool {
    let dirs: Vec<std::path::PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    let exts: Vec<String> = if cfg!(windows) {
        std::env::var("PATHEXT").unwrap_or_else(|_| ".EXE;.CMD;.BAT".into()).split(';').map(|e| e.to_lowercase()).collect()
    } else {
        vec![String::new()]
    };
    dirs.iter().any(|d| exts.iter().any(|e| d.join(format!("{name}{e}")).is_file()))
}

/// Windows shell for a new terminal from the `shell` setting (auto|pwsh|powershell|bash|wsl|cmd).
/// `auto` and unknown values prefer PowerShell 7 when installed, else Windows PowerShell 5.1.
fn windows_shell(setting: &str, has_pwsh: bool) -> (String, Vec<String>) {
    let ps = |p: &str| (p.to_string(), vec!["-NoLogo".to_string()]);
    match setting {
        "pwsh" => ps("pwsh"),
        "powershell" => ps("powershell"),
        "bash" => ("bash".into(), vec!["-l".into()]),
        "wsl" => ("wsl".into(), vec![]),
        "cmd" => ("cmd".into(), vec![]),
        _ if has_pwsh => ps("pwsh"),
        _ => ps("powershell"),
    }
}

/// What the UI shows in the diagnostics panel: versions, where the state lives, what is missing.
#[tauri::command]
async fn diagnostics(state: tauri::State<'_, Arc<State>>) -> Result<Value, CallError> {
    let st = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let daemon = with_client(&st.bg, true, |c| {
            Ok((c.proto, c.instance, c.build.clone(), c.security, [c.version.clone(), c.commit.clone(), c.built.clone()]))
        })
        .ok();
        let reported = |i: usize| daemon.as_ref().map(|d| d.4[i].clone()).filter(|s| !s.is_empty());
        // the offer stands only for the daemon run it was made for
        let offer = match (*STALE.lock_recover(), &daemon) {
            (Some((instance, stale::Decision::Offer(r))), Some(d)) if d.1 == instance => json!({"reason": r.as_str()}),
            _ => Value::Null,
        };
        let lag = proto::data_dir().join("lag.log");
        json!({
            "app_version": env!("CARGO_PKG_VERSION"),
            "app_commit": env!("AXELSTRA_GIT_SHA"),
            "app_built": env!("AXELSTRA_BUILD_DATE"),
            "os": format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
            "webview": tauri::webview_version().ok(),
            "app_proto": proto::PROTO,
            "daemon_connected": daemon.is_some(),
            "daemon_proto": daemon.as_ref().map(|d| d.0),
            "daemon_instance": daemon.as_ref().map(|d| d.1),
            "daemon_build": daemon.as_ref().map(|d| d.2.clone()),
            "daemon_security": daemon.as_ref().map(|d| d.3),
            "daemon_version": reported(0),
            "daemon_commit": reported(1),
            "daemon_built": reported(2),
            "daemon_offer": offer,
            "app_security_min": proto::SECURITY_MIN,
            "data_dir": proto::data_dir().to_string_lossy(),
            "lag_log": lag.to_string_lossy(),
            "lag_log_bytes": std::fs::metadata(&lag).map(|m| m.len()).unwrap_or(0),
            "missing_tools": missing_tools(),
        })
    })
    .await
    .map_err(|e| CallError::from(e.to_string()))
}

/// Plain-text bundle for bug reports: version, OS, data dir, tail of the latest log and of crash.log.
#[tauri::command]
async fn diagnostics_bundle() -> Result<String, CallError> {
    tauri::async_runtime::spawn_blocking(|| logging::bundle(&proto::data_dir())).await.map_err(|e| CallError::from(e.to_string()))
}

/// An error caught by the webview (window.onerror / unhandledrejection), written to the log file.
#[tauri::command]
fn log_frontend_error(kind: String, message: String, stack: Option<String>) {
    logging::log_frontend(&kind, &message, stack.as_deref().unwrap_or(""));
}

/// Where pasted clipboard images go: a folder of the per-user data dir (never the shared temp dir, where another
/// local user could plant links), private to the user on Unix.
fn paste_dir() -> std::path::PathBuf {
    proto::data_dir().join("paste")
}

/// Deletes pasted clipboard images older than a day.
fn clean_pastes() {
    let Ok(rd) = std::fs::read_dir(paste_dir()) else { return };
    let cutoff = Duration::from_secs(24 * 3600);
    for e in rd.flatten() {
        let old = e.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).map(|age| age > cutoff).unwrap_or(false);
        if old && e.path().extension().is_some_and(|x| x == "png") {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

/// A hard-to-guess file stem: the std hasher is randomly keyed per instance, mixed with the clock and process id.
fn random_stem() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    let ns = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    h.write_u128(ns);
    h.write_u32(std::process::id());
    format!("paste-{:016x}", h.finish())
}

/// Creates a new PNG file in the paste dir; `create_new` refuses an existing name or a planted link.
fn create_paste_file(dir: &std::path::Path) -> Result<(std::fs::File, std::path::PathBuf), CallError> {
    for _ in 0..8 {
        let path = dir.join(format!("{}.png", random_stem()));
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(f) => return Ok((f, path)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(CallError::from(e.to_string())),
        }
    }
    Err(CallError::from("could not create a paste file".to_string()))
}

/// The terminal to start: this machine's, or (with `host`) the login shell of that remote host.
#[tauri::command]
async fn default_shell(host: Option<String>) -> Result<(String, Vec<String>), CallError> {
    let Some(host) = host.filter(|h| h != hosts::LOCAL) else { return Ok(local_shell()) };
    tauri::async_runtime::spawn_blocking(move || {
        let h = hosts::get(&host).ok_or_else(|| host_err!("app.host_unknown", "unknown host: {host}", host = host))?;
        let info = hosts::with_remote(&h, hosts::Which::Bg, true, |c| c.call("host.info", json!({}))).map_err(|e| CallError::from(&e))?;
        // only a shell the daemon would start for a terminal; anything else falls back to sh
        let shell = info["shell"].as_str().filter(|s| is_shell(s)).unwrap_or("/bin/sh").to_string();
        Ok((shell, vec!["-l".to_string()]))
    })
    .await
    .map_err(|e| e.to_string())?
}

fn local_shell() -> (String, Vec<String>) {
    if cfg!(windows) {
        windows_shell(setting("shell").as_str().unwrap_or("auto"), on_path("pwsh"))
    } else {
        let sh = std::env::var("SHELL").unwrap_or_else(|_| if cfg!(target_os = "macos") { "/bin/zsh".into() } else { "/bin/bash".into() });
        // a login shell loads the user's PATH (Homebrew, nvm...), which a GUI app does not inherit
        (sh, vec!["-l".into()])
    }
}

/// What the UI shows about a pending update. Nothing from the update server is passed on
/// except the version and notes text.
#[derive(serde::Serialize)]
struct UpdateInfo {
    version: String,
    notes: Option<String>,
}

/// Native clipboard: WebView2 gates `navigator.clipboard` behind a permission prompt.
/// Images are saved as PNG in the user's data dir and returned as a path to type into the terminal.
#[tauri::command]
fn clipboard_read() -> Result<Value, CallError> {
    let mut cb = arboard::Clipboard::new().map_err(|e| e.to_string())?;
    if let Ok(img) = cb.get_image() {
        let dir = paste_dir();
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
        }
        let (file, path) = create_paste_file(&dir)?;
        let mut enc = png::Encoder::new(std::io::BufWriter::new(file), img.width as u32, img.height as u32);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut w = enc.write_header().map_err(|e| e.to_string())?;
        w.write_image_data(&img.bytes).map_err(|e| e.to_string())?;
        w.finish().map_err(|e| e.to_string())?;
        return Ok(json!({ "kind": "image", "value": path.to_string_lossy() }));
    }
    match cb.get_text() {
        Ok(t) if !t.is_empty() => Ok(json!({ "kind": "text", "value": t })),
        _ => Ok(json!({ "kind": "none", "value": "" })),
    }
}

/// Hands a resolved `file://` url to the system opener. With `AXELSTRA_TEST_OPENER_LOG` set (tests, E2E) the url is
/// appended to that file instead and no browser starts.
fn open_resolved(url: &str, open: impl FnOnce(&str) -> Result<(), String>) -> Result<(), CallError> {
    if let Some(log) = std::env::var_os("AXELSTRA_TEST_OPENER_LOG").filter(|l| !l.is_empty()) {
        use std::io::Write;
        return std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log)
            .and_then(|mut f| writeln!(f, "{url}"))
            .map_err(|e| CallError::from(e.to_string()));
    }
    open(url).map_err(|e| host_err!("files.html_open_failed", "could not open {path}: {detail}", path = url, detail = e))
}

/// Ctrl+click on an HTML path in a terminal: the daemon resolves and confines it (`files.resolve_html`), then the system
/// opener gets the url *it* returned. The page never passes a url, so it cannot open an arbitrary address or program.
/// `session` is a session of this machine's daemon (a remote host's files are not on this disk).
#[tauri::command]
async fn open_html_file(app: tauri::AppHandle, state: tauri::State<'_, Arc<State>>, session: u32, path: String) -> Result<(), CallError> {
    let st = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let r = with_client(&st.control, true, |c| c.call("files.resolve_html", json!({"session": session, "path": path})))
            .map_err(|e| CallError::from(&e))?;
        let url = r["url"].as_str().unwrap_or_default().to_string();
        if !url.starts_with("file://") {
            return Err(host_err!("files.html_not_allowed", "not allowed to open {path}", path = path));
        }
        open_resolved(&url, |u| {
            use tauri_plugin_opener::OpenerExt;
            app.opener().open_url(u, None::<&str>).map_err(|e| e.to_string())
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
fn clipboard_write(text: String) -> Result<(), CallError> {
    arboard::Clipboard::new().and_then(|mut c| c.set_text(text)).map_err(|e| CallError::from(e.to_string()))
}

/// `None` = this build is the latest. Errors become one fixed sentence: the plugin's own
/// error can quote whatever the endpoint answered.
#[tauri::command]
async fn update_check(app: tauri::AppHandle) -> Result<Option<UpdateInfo>, CallError> {
    use tauri_plugin_updater::UpdaterExt;
    let updater = app.updater().map_err(|_| host_err!("app.updater_unavailable", "updater unavailable"))?;
    match updater.check().await {
        Ok(Some(u)) => Ok(Some(UpdateInfo { version: u.version, notes: u.body })),
        Ok(None) => Ok(None),
        Err(_) => Err(host_err!("app.update_check_failed", "update check failed")),
    }
}

/// Downloads and verifies the update in the background and keeps it for later. `expected` is the
/// version the user was told about: if a newer release landed since, nothing is kept.
#[tauri::command]
async fn update_download(app: tauri::AppHandle, state: tauri::State<'_, Arc<State>>, expected: String) -> Result<(), CallError> {
    use tauri_plugin_updater::UpdaterExt;
    if let Some((u, _)) = state.pending.lock_recover().as_ref() {
        if u.version == expected {
            return Ok(());
        }
    }
    let updater = app.updater().map_err(|_| host_err!("app.updater_unavailable", "updater unavailable"))?;
    let update = updater
        .check()
        .await
        .map_err(|_| host_err!("app.update_check_failed", "update check failed"))?
        .ok_or_else(|| host_err!("app.up_to_date", "already up to date"))?;
    if update.version != expected {
        return Err(host_err!("app.update_newer", "a newer release ({version}) appeared; check again", version = update.version));
    }
    let bytes = update
        .download(|_, _| {}, || {})
        .await
        .map_err(|_| host_err!("app.update_download_failed", "download or signature check failed"))?;
    *state.pending.lock_recover() = Some((update, bytes));
    Ok(())
}

/// Installs the downloaded update and restarts the window. The installer renames the running daemon
/// instead of killing it, so terminals and agents keep running through the update.
#[tauri::command]
fn update_apply(app: tauri::AppHandle, state: tauri::State<'_, Arc<State>>) -> Result<(), CallError> {
    let (update, bytes) = state.pending.lock_recover().take().ok_or("no update downloaded")?;
    update.restart_after_install(true).install(bytes).map_err(|_| "install failed".to_string())?;
    app.restart();
}

/// Browser switches for the test drivers (`tools/e2e.mjs`), debug builds only: `AXELSTRA_TEST_DEVTOOLS_PORT=N` opens the
/// DevTools port, `AXELSTRA_TEST_BROWSER_ARGS` adds more switches. wry always hands WebView2 its own default switches through
/// the API, and on current runtimes (155) that makes `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS` be ignored, so the host passes them
/// itself. Release builds never read the variables: no debugging port can be opened in production.
#[cfg(all(debug_assertions, windows))]
fn test_browser_args() -> Option<String> {
    let port = std::env::var("AXELSTRA_TEST_DEVTOOLS_PORT").ok()?;
    let port: u16 = port.trim().parse().ok().filter(|p| *p != 0)?;
    let mut args = format!("--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --remote-debugging-port={port}");
    if let Ok(extra) = std::env::var("AXELSTRA_TEST_BROWSER_ARGS") {
        args.push(' ');
        args.push_str(extra.trim());
    }
    Some(args)
}

/// The main window comes from tauri.conf.json (`create: false`) so the host can add the test switches above.
fn create_main_window(app: &tauri::AppHandle) -> tauri::Result<()> {
    let Some(cfg) = app.config().app.windows.first().cloned() else { return Ok(()) };
    #[allow(unused_mut)]
    let mut builder = tauri::WebviewWindowBuilder::from_config(app, &cfg)?;
    #[cfg(all(debug_assertions, windows))]
    if let Some(args) = test_browser_args() {
        builder = builder.additional_browser_args(&args);
    }
    let hidden = std::env::var_os("AXELSTRA_TEST_HIDDEN").is_some_and(|v| v == "1");
    if hidden {
        // never take the focus, not even while the webview initialises
        builder = builder.focused(false);
    }
    let w = builder.build()?;
    if hidden {
        make_inert(&w);
    }
    Ok(())
}

/// Test-hidden window: fully transparent, click-through, tool window and never activatable. Applied right after
/// creation (before the webview gets to focus its host), and again before showing. WebView2 counts it as visible (an
/// off-screen or minimised window is occluded and never renders) while the user sees and touches nothing.
#[cfg(windows)]
fn make_inert(w: &tauri::WebviewWindow) {
    use windows_sys::Win32::UI::WindowsAndMessaging as wm;
    let _ = w.set_skip_taskbar(true);
    if let Ok(h) = w.hwnd() {
        unsafe {
            let ex = wm::GetWindowLongPtrW(h.0 as _, wm::GWL_EXSTYLE);
            let add = (wm::WS_EX_LAYERED | wm::WS_EX_TRANSPARENT | wm::WS_EX_TOOLWINDOW | wm::WS_EX_NOACTIVATE) as isize;
            wm::SetWindowLongPtrW(h.0 as _, wm::GWL_EXSTYLE, ex | add);
            wm::SetLayeredWindowAttributes(h.0 as _, 0, 0, wm::LWA_ALPHA);
        }
    }
}
#[cfg(not(windows))]
fn make_inert(_: &tauri::WebviewWindow) {}

/// The window is created hidden (`visible: false` in tauri.conf.json) and shown here. Under `AXELSTRA_TEST_HIDDEN=1`
/// (visual/e2e drivers) it is shown fully transparent, click-through, out of the taskbar and never activated: WebView2 still renders, so
/// the DevTools screenshots work, but nothing appears on the user's screen or steals the focus.
fn show_main_window(app: &tauri::AppHandle) {
    use tauri::Manager;
    let Some(w) = app.get_webview_window("main") else { return };
    if std::env::var_os("AXELSTRA_TEST_HIDDEN").is_some_and(|v| v == "1") {
        make_inert(&w);
        #[cfg(windows)]
        if let Ok(h) = w.hwnd() {
            // shown without activating: the foreground window of the user stays where it is
            use windows_sys::Win32::UI::WindowsAndMessaging as wm;
            unsafe {
                wm::ShowWindow(h.0 as _, wm::SW_SHOWNOACTIVATE);
            }
            return;
        }
    }
    let _ = w.show();
}

fn main() {
    logging::init(&proto::data_dir());
    gpu::prefer_best_gpu();
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .manage(Arc::new(State::default()))
        .manage(voice::Voice::default())
        .setup(|app| {
            let _ = APP.set(app.handle().clone());
            create_main_window(app.handle())?;
            show_main_window(app.handle());
            hosts::load();
            std::thread::spawn(clean_pastes);
            Ok(())
        })
        // closing the window asks the UI first (it shows the confirmation, or quits when the user opted out)
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "main" {
                    api.prevent_close();
                    use tauri::Emitter;
                    let _ = window.emit("close-requested", ());
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            missing_tools,
            daemon_call,
            daemon_call_bg,
            project_add_pick,
            folder_pick,
            backup_restore_pick,
            workspace_export_pick,
            workspace_import_pick,
            workspace_map_pick,
            workspace_map_clear,
            workspace_import_apply,
            workspace_import_cancel,
            session_write,
            lag_log,
            session_attach,
            session_detach,
            watch_start,
            watch_set,
            default_shell,
            hosts_list,
            host_add,
            host_remove,
            host_reconnect,
            project_add_remote,
            update_check,
            update_download,
            update_apply,
            clipboard_read,
            clipboard_write,
            open_html_file,
            localfs::local_fs_list,
            localfs::local_fs_cancel,
            localfs::local_fs_stat,
            localfs::local_fs_read,
            localfs::local_fs_act,
            localfs::local_fs_watch,
            notify_session,
            app_quit,
            diagnostics,
            diagnostics_bundle,
            log_frontend_error,
            stats::keep_awake,
            stats::mem_usage,
            voice::voice_status,
            voice::voice_download,
            voice::voice_transcribe
        ])
        .build(tauri::generate_context!())
        .expect("error while building axelstra")
        .run(|app, event| {
            // a downloaded update is installed when the window is closed, without relaunching the app
            if let tauri::RunEvent::ExitRequested { .. } = event {
                let state = app.state::<Arc<State>>();
                let pending = state.pending.lock_recover().take();
                if let Some((update, bytes)) = pending {
                    let _ = update.restart_after_install(false).install(bytes);
                }
            }
        });
}

#[cfg(test)]
mod tests {
    use super::{check_allowed, windows_shell, ALLOWED_METHODS};
    use std::collections::BTreeSet;
    use std::path::Path;

    #[test]
    fn auto_prefers_pwsh() {
        assert_eq!(windows_shell("auto", true).0, "pwsh");
        assert_eq!(windows_shell("auto", false).0, "powershell");
        assert_eq!(windows_shell("nonsense", true).0, "pwsh");
    }

    #[test]
    fn explicit_choice_wins() {
        assert_eq!(windows_shell("powershell", true).0, "powershell");
        assert_eq!(windows_shell("cmd", true), ("cmd".into(), vec![]));
        assert_eq!(windows_shell("wsl", false).0, "wsl");
        assert_eq!(windows_shell("bash", false).1, vec!["-l".to_string()]);
    }

    fn sources(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                sources(&p, out);
            } else if p.extension().is_some_and(|x| x == "ts" || x == "tsx") && !p.file_name().unwrap().to_string_lossy().contains(".test.")
            {
                out.push(p);
            }
        }
    }

    /// Every `call("x.y"` / `callBg("x.y"` / `callOr("x.y"` in the frontend (an optional `<Type>` after the name is skipped).
    /// A call whose method is not a string literal is returned as `?file`, since the list cannot cover it.
    fn called_methods() -> BTreeSet<String> {
        let mut files = vec![];
        sources(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../src"), &mut files);
        assert!(!files.is_empty(), "frontend sources not found");
        let mut found = BTreeSet::new();
        for f in files {
            let t = std::fs::read_to_string(&f).unwrap();
            let b = t.as_bytes();
            for name in ["call", "callBg", "callOr"] {
                for (at, _) in t.match_indices(name) {
                    let ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_' || c == b'$' || c == b'.';
                    if (at > 0 && ident(b[at - 1])) || !matches!(b.get(at + name.len()), Some(b'(' | b'<')) {
                        continue;
                    }
                    let mut i = at + name.len();
                    if b[i] == b'<' {
                        let mut depth = 0;
                        while i < b.len() {
                            match b[i] {
                                b'<' => depth += 1,
                                b'>' if b[i - 1] != b'=' => {
                                    depth -= 1;
                                    if depth == 0 {
                                        break;
                                    }
                                }
                                _ => {}
                            }
                            i += 1;
                        }
                        i += 1;
                    }
                    if b.get(i) != Some(&b'(') {
                        continue;
                    }
                    let rest = t[i + 1..].trim_start();
                    let arg = rest.chars().next();
                    // the declarations themselves: `call = <T>(method: string` has no literal and no `(` right after the name
                    match arg {
                        Some(q @ ('"' | '\'' | '`')) => {
                            let end = rest[1..].find(q).expect("unterminated string");
                            found.insert(rest[1..1 + end].to_string());
                        }
                        // `callOr` forwards its own `method` parameter to `call`: the literal is at its callers
                        _ if rest.starts_with("method,") => {}
                        _ => {
                            found.insert(format!("?{}", f.file_name().unwrap().to_string_lossy()));
                        }
                    }
                }
            }
        }
        found
    }

    #[test]
    fn allow_list_matches_frontend_calls() {
        let called = called_methods();
        let dynamic: Vec<_> = called.iter().filter(|m| m.starts_with('?')).collect();
        assert!(dynamic.is_empty(), "daemon call with a non-literal method (cannot be allow-listed): {dynamic:?}");
        let listed: BTreeSet<String> = ALLOWED_METHODS.iter().map(|m| m.to_string()).collect();
        assert_eq!(listed.len(), ALLOWED_METHODS.len(), "duplicate in ALLOWED_METHODS");
        let missing: Vec<_> = called.difference(&listed).collect();
        assert!(missing.is_empty(), "frontend calls methods missing from ALLOWED_METHODS (main.rs): {missing:?}");
        let unused: Vec<_> = listed.difference(&called).collect();
        assert!(unused.is_empty(), "ALLOWED_METHODS lists methods the frontend never calls: {unused:?}");
    }

    #[test]
    fn spawn_and_write_are_narrowed() {
        use super::check_call;
        use serde_json::json;
        for ok in ["pwsh", "powershell", "C:\\Program Files\\PowerShell\\7\\pwsh.exe", "/bin/zsh", "/usr/bin/bash", "CMD.EXE", "wsl"] {
            assert!(check_call("session.spawn", &json!({"program": ok})).is_ok(), "{ok}");
        }
        for bad in ["calc.exe", "/usr/bin/python3", "curl", "", "C:\\Windows\\System32\\mshta.exe", "bash.exe.evil"] {
            assert!(check_call("session.spawn", &json!({"program": bad})).is_err(), "{bad}");
        }
        assert!(check_call("session.spawn", &json!({"program": "bash", "env": {"AXELSTRA_KEY": "x"}})).is_ok());
        assert!(check_call("session.spawn", &json!({"program": "bash", "env": {"LD_PRELOAD": "x"}})).is_err());
        assert!(check_call("fs.write", &json!({"path": "src/a.rs"})).is_ok());
        // "C:\x" is a drive path only on Windows
        let drive = ["C:\\x"].into_iter().filter(|_| cfg!(windows));
        for bad in ["", "../x", "/etc/passwd", ".git/config", ".GIT\\hooks\\pre-commit", "a/.git/b", "a/../b"].into_iter().chain(drive) {
            assert!(check_call("fs.write", &json!({"path": bad})).is_err(), "{bad}");
        }
        assert!(check_call("fs.read", &json!({"path": "../x"})).is_ok());
        assert!(check_call("orch.start", &json!({"project": "p", "max": 3})).is_ok());
        assert!(check_call("orch.start", &json!({"project": "p", "agent": ["calc.exe"]})).is_err());
    }

    #[test]
    fn path_taking_methods_only_through_the_native_pick() {
        use super::check_call;
        use serde_json::json;
        assert!(!ALLOWED_METHODS.contains(&"project.add"));
        assert!(!ALLOWED_METHODS.contains(&"backup.restore"));
        assert!(check_call("project.add", &json!({"path": "/home/x/proj"})).is_err());
        assert!(check_call("backup.restore", &json!({"path": "/tmp/evil"})).is_err());
        // the workspace file and the folder map come from native dialogs held by the host: the page cannot name them
        for m in ["workspace.export", "workspace.import_preview", "workspace.import"] {
            assert!(!ALLOWED_METHODS.contains(&m), "{m}");
            assert!(check_call(m, &json!({"path": "/tmp/evil"})).is_err(), "{m}");
        }
        assert!(check_call("project.list", &json!({})).is_ok());
        assert!(check_call("backup.export", &json!({})).is_ok());
    }

    #[test]
    fn opener_log_replaces_the_browser_in_tests() {
        let log = std::env::temp_dir().join(format!("ax-opener-{}.log", std::process::id()));
        let _ = std::fs::remove_file(&log);
        std::env::set_var("AXELSTRA_TEST_OPENER_LOG", &log);
        let res = super::open_resolved("file:///a/b.html", |_| panic!("must not open a browser"));
        assert!(res.is_ok());
        assert!(super::open_resolved("file:///c%20d.html", |_| panic!("no")).is_ok());
        std::env::remove_var("AXELSTRA_TEST_OPENER_LOG");
        assert_eq!(
            std::fs::read_to_string(&log).unwrap(),
            "file:///a/b.html
file:///c%20d.html
"
        );
        let _ = std::fs::remove_file(&log);
        assert!(super::open_resolved("file:///x.html", |_| Err("boom".into())).is_err());
    }

    #[test]
    fn unknown_methods_are_refused() {
        assert!(check_allowed("session.spawn").is_ok());
        assert!(check_allowed("session.pending_clear").is_err());
        assert!(check_allowed("").is_err());
        assert!(check_allowed("flow.set").unwrap_err().message.contains("not allowed"));
    }
}

/// `connect_via` against a fake daemon on a temp pipe: what it does with a stale daemon. Nothing real is started.
#[cfg(test)]
mod stale_connect {
    use super::{connect_via, proto, stale, Link};
    use serde_json::{json, Value};
    use std::io::{Read, Write};
    use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    const APP: &str = "aaaaaaa";

    /// What the fake daemon answers and what it was asked.
    struct Fake {
        instance: AtomicU64,
        commit: Mutex<Option<String>>,
        rows: Value,
        gone: AtomicBool,
        shutdowns: AtomicUsize,
        lists: AtomicUsize,
    }

    impl Fake {
        fn new(commit: Option<&str>, rows: Value) -> Arc<Fake> {
            Arc::new(Fake {
                instance: AtomicU64::new(1),
                commit: Mutex::new(commit.map(String::from)),
                rows,
                gone: AtomicBool::new(false),
                shutdowns: AtomicUsize::new(0),
                lists: AtomicUsize::new(0),
            })
        }
    }

    fn handle(mut conn: impl Read + Write, fake: Arc<Fake>) {
        // a daemon that shut down answers nobody: the connection closes at once
        if fake.gone.load(Ordering::SeqCst) {
            return;
        }
        loop {
            let mut len = [0u8; 4];
            if conn.read_exact(&mut len).is_err() {
                return;
            }
            let mut buf = vec![0u8; u32::from_le_bytes(len) as usize];
            if conn.read_exact(&mut buf).is_err() {
                return;
            }
            let req: Value = serde_json::from_slice(&buf[1..]).unwrap();
            let result = match req["method"].as_str().unwrap_or_default() {
                "hello" => {
                    let mut h = json!({"proto": proto::PROTO, "security": proto::SECURITY_MIN,
                        "instance": fake.instance.load(Ordering::SeqCst), "version": "1.0.0"});
                    if let Some(c) = fake.commit.lock().unwrap().clone() {
                        h["commit"] = json!(c);
                    }
                    h
                }
                "session.list" => {
                    fake.lists.fetch_add(1, Ordering::SeqCst);
                    fake.rows.clone()
                }
                "daemon.shutdown" => {
                    fake.shutdowns.fetch_add(1, Ordering::SeqCst);
                    fake.gone.store(true, Ordering::SeqCst);
                    json!({})
                }
                _ => json!({}),
            };
            let body = json!({"id": req["id"], "result": result});
            let _ = conn.write_all(&proto::encode(proto::JSON, body.to_string().as_bytes()));
            if fake.gone.load(Ordering::SeqCst) {
                return;
            }
        }
    }

    /// Serves `pipe` until the test process ends; returns once the first instance listens.
    #[cfg(windows)]
    fn serve(pipe: &str, fake: Arc<Fake>) {
        use std::os::windows::io::FromRawHandle;
        use windows_sys::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE};
        use windows_sys::Win32::System::Pipes::{ConnectNamedPipe, CreateNamedPipeW};
        let wide: Vec<u16> = pipe.encode_utf16().chain(std::iter::once(0)).collect();
        // PIPE_ACCESS_DUPLEX, byte mode, blocking, unlimited instances
        let create = move || unsafe { CreateNamedPipeW(wide.as_ptr(), 3, 0, 255, 65536, 65536, 0, std::ptr::null()) as usize };
        let first = create();
        assert_ne!(first, INVALID_HANDLE_VALUE as usize);
        std::thread::spawn(move || {
            let mut h = first;
            loop {
                // fails with ERROR_PIPE_CONNECTED when the client came first: connected all the same
                unsafe { ConnectNamedPipe(h as HANDLE, std::ptr::null_mut()) };
                let f = unsafe { std::fs::File::from_raw_handle(h as _) };
                let fake = fake.clone();
                std::thread::spawn(move || handle(f, fake));
                h = create();
                if h == INVALID_HANDLE_VALUE as usize {
                    return;
                }
            }
        });
    }

    #[cfg(unix)]
    fn serve(pipe: &str, fake: Arc<Fake>) {
        let l = std::os::unix::net::UnixListener::bind(pipe).unwrap();
        std::thread::spawn(move || {
            for s in l.incoming().flatten() {
                let fake = fake.clone();
                std::thread::spawn(move || handle(s, fake));
            }
        });
    }

    fn pipe_for(name: &str) -> String {
        if cfg!(windows) {
            format!(r"\\.\pipe\axelstra-stale-test-{}-{name}", std::process::id())
        } else {
            let p = std::env::temp_dir().join(format!("axs-{}-{name}.sock", std::process::id()));
            let _ = std::fs::remove_file(&p);
            p.to_string_lossy().into_owned()
        }
    }

    type Seen = [(u64, Option<stale::Decision>); 2];

    /// Runs `connect_via` twice against a fake daemon with `commit` and `rows`. The daemon exe is beside the app exe.
    /// The start function plays the app's own daemon: a new run on the same pipe, with the app's commit.
    fn run(name: &str, commit: Option<&str>, rows: Value) -> (Arc<Fake>, bool, Seen, Option<(u64, stale::Decision)>) {
        let pipe = pipe_for(name);
        let fake = Fake::new(commit, rows);
        serve(&pipe, fake.clone());
        let dir = std::env::temp_dir();
        let started = AtomicBool::new(false);
        let start = || {
            started.store(true, Ordering::SeqCst);
            fake.instance.store(2, Ordering::SeqCst);
            *fake.commit.lock().unwrap() = Some(APP.into());
            fake.gone.store(false, Ordering::SeqCst);
            Ok(None)
        };
        let exe = dir.join("axelstra.exe");
        let daemon_exe = |_: &axelstra_daemon::client::Client| Some(exe.clone());
        let memo = Mutex::new(None);
        let link = Link {
            pipe,
            token: "t".into(),
            app: stale::Build { version: "1.0.1".into(), commit: APP.into() },
            app_exe: Some(dir.join("axelstra-app.exe")),
            daemon_exe: &daemon_exe,
            start: &start,
            memo: &memo,
        };
        let (a, fa) = connect_via(&link).unwrap();
        let (b, fb) = connect_via(&link).unwrap();
        let m = *memo.lock().unwrap();
        (fake.clone(), started.load(Ordering::SeqCst), [(a.instance, fa), (b.instance, fb)], m)
    }

    fn idle() -> Value {
        json!([{"id": 1, "running": true, "idle_ms": 600000, "waiting": false, "wait": null, "awaiting": null, "paused": null, "hibernated": null, "meta": {"agent": "claude"}}])
    }

    #[test]
    fn stale_and_idle_is_replaced_once() {
        let (fake, started, seen, memo) = run("idle", Some("bbbbbbb"), idle());
        assert_eq!(fake.shutdowns.load(Ordering::SeqCst), 1);
        assert!(started, "the replacement was not started");
        // the first connection lands on the new run; the second one reuses it without judging again
        assert_eq!(seen, [(2, None), (2, None)]);
        assert_eq!(fake.lists.load(Ordering::SeqCst), 1);
        assert_eq!(memo, Some((2, stale::Decision::Keep)));
    }

    #[test]
    fn stale_and_busy_is_kept_and_offered_once() {
        let mut rows = idle();
        rows[0]["awaiting"] = json!({"reason": "CI", "since": 1, "until": null});
        let (fake, started, seen, memo) = run("busy", Some("bbbbbbb"), rows);
        assert_eq!(fake.shutdowns.load(Ordering::SeqCst), 0);
        assert!(!started);
        let offer = stale::Decision::Offer(stale::Reason::Busy);
        // only the first connection decides (and would log and emit); the second reuses the offer
        assert_eq!(seen, [(1, Some(offer)), (1, None)]);
        assert_eq!(fake.lists.load(Ordering::SeqCst), 1);
        assert_eq!(memo, Some((1, offer)));
    }

    #[test]
    fn a_stale_daemon_with_a_terminal_open_is_kept() {
        let mut rows = idle();
        rows[0]["meta"] = json!({});
        let (fake, started, _, memo) = run("term", Some("bbbbbbb"), rows);
        assert_eq!((fake.shutdowns.load(Ordering::SeqCst), started), (0, false));
        assert_eq!(memo, Some((1, stale::Decision::Offer(stale::Reason::Busy))));
    }

    #[test]
    fn same_commit_is_kept() {
        let (fake, started, seen, memo) = run("same", Some(APP), idle());
        assert_eq!((fake.shutdowns.load(Ordering::SeqCst), started, fake.lists.load(Ordering::SeqCst)), (0, false, 0));
        assert_eq!(seen, [(1, Some(stale::Decision::Keep)), (1, None)]);
        assert_eq!(memo, Some((1, stale::Decision::Keep)));
    }

    #[test]
    fn an_old_daemon_without_a_commit_is_kept() {
        let (fake, started, seen, _) = run("old", None, idle());
        assert_eq!((fake.shutdowns.load(Ordering::SeqCst), started, fake.lists.load(Ordering::SeqCst)), (0, false, 0));
        assert_eq!(seen, [(1, Some(stale::Decision::Keep)), (1, None)]);
    }
}
