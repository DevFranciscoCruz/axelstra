use crate::proto::*;
use crate::store::{read_json, write_atomic, write_json, LockRecover};
use anyhow::Result;
use axelstra_core::paths::{is_within, norm};
use axelstra_core::pty::{Session, Spawn};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::{Duration, Instant};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::mpsc;
mod containers;
mod files;
mod hibernate;
mod history;
mod loops;
mod messaging;
mod misc;
mod orchestration;
mod origin;
mod registry;
mod removal;
mod session;
mod sources;
mod triggers;
mod vcs;
mod worktrees;

use history::*;
use loops::*;
use registry::*;

/// Identifies this daemon run: a client that sees it change knows the daemon restarted (its sessions are gone).
static INSTANCE: std::sync::LazyLock<u64> = std::sync::LazyLock::new(|| {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
});

/// Frames queued for one connection's writer task. Bounded: a client that stops reading slows its own senders
/// down (they block) instead of growing the daemon's memory.
type Tx = mpsc::Sender<Vec<u8>>;
const OUT_QUEUE: usize = 1024;

/// A terminal size from a request: `as u16` would wrap 65537 to 1, and 0 is no size at all.
fn term_dim(v: &Value, default: u16) -> u16 {
    v.as_u64().map_or(default, |x| x.clamp(1, 1000) as u16)
}

/// What `hello` and `ping` say about this daemon, so the app can tell when it is older than it should be.
fn hello_info() -> Value {
    json!({"proto": PROTO, "instance": *INSTANCE, "build": BUILD_ID, "security": SECURITY_MIN,
        "version": axelstra_core::buildinfo::VERSION, "commit": axelstra_core::buildinfo::COMMIT, "built": axelstra_core::buildinfo::BUILT})
}

/// Calls that only the app (the user) may make; a connection with role=agent is refused them.
fn agent_forbidden(method: &str, p: &Value) -> bool {
    match method {
        "computer.enable" => p["on"].as_bool() == Some(true),
        // an agent must not raise its own limit or switch the auto-pause off; the events are the user's to take
        "budget.check" | "budget.events" => true,
        // they list the file system outside any worktree
        "host.info" | "host.browse" => true,
        // notifications, the stall watchdog, installs and the shell are what keeps the user informed and safe
        "settings.set" => ["budgets", "hosts", "notify_mode", "notify_sound", "stall_minutes", "auto_install", "shell", "hibernate"]
            .iter()
            .any(|k| p.get(*k).is_some()),
        // the saved sessions are the user's to drop, and an agent must not stop the daemon or switch the clean-up off
        "daemon.shutdown" | "session.pending_clear" => true,
        // hibernating and waking are the user's; an agent only asks to `await` for itself
        "session.hibernate" | "session.wake" => true,
        "janitor.auto" => p.get("on").is_some(),
        "project.add"
        | "project.remove"
        | "project.remove_preview"
        | "project.setup"
        | "hooks.install"
        | "automation.save"
        | "orch.start"
        | "notices.take"
        | "audit.list"
        | "backup.export"
        | "backup.restore"
        | "gh.send_comment"
        | "gh.pr_refresh"
        | "triggers.set"
        | "triggers.events"
        | "worktree.rebase"
        | "worktree.integrate"
        | "integrate.undo"
        | "orch.cancel"
        | "project.integrate"
        | "project.discover"
        | "project.add_discovered"
        | "project.clone"
        | "project.clone_status"
        | "project.clone_cancel"
        | "project.init"
        | "gh.repos"
        | "workspace.export"
        | "workspace.import_preview"
        | "workspace.import"
        // opening a browser is the user's act: the app host calls it, never an agent
        | "files.resolve_html"
        // describing, revealing, opening or RUNNING a path: the user confirmed it in the app, an agent must never
        | "files.resolve_path"
        | "files.path_action" => true,
        _ => false,
    }
}

/// True when the agent session `me` may drive `target`: it is `me` itself or a session `me` started, found by walking
/// the bus parent chain up from `target`. A lead keeps driving its workers; a worker cannot touch its siblings or parents.
fn agent_drives(reg: &Registry, me: u32, target: u32) -> bool {
    let mut cur = target;
    // the chain is short; the bound only protects against a cycle
    for _ in 0..64 {
        if cur == me {
            return true;
        }
        match reg.bus.parent_of(cur) {
            Some(parent) => cur = parent,
            None => return false,
        }
    }
    false
}

/// An agent connection (`agent` is its session) may name only its own session in `session`; the app and the user's CLI
/// (`agent` is `None`) may name any.
pub(crate) fn require_own(agent: Option<u32>, session: Option<u64>, method: &str) -> Result<()> {
    match agent {
        Some(me) if session != Some(u64::from(me)) => {
            Err(axelstra_core::coded!("rpc.agent_own_session", "{method} is only allowed for the agent's own session", method = method))
        }
        _ => Ok(()),
    }
}

/// The refusal an agent gets when it reaches for a session that is not its own or one of its descendants.
fn not_own_tree(method: &str) -> anyhow::Error {
    axelstra_core::coded!(
        "rpc.agent_own_tree",
        "{method} is only allowed for the agent's own session and the sessions it started",
        method = method
    )
}

pub(crate) const SHELLS: [&str; 6] = ["auto", "pwsh", "powershell", "bash", "wsl", "cmd"];
pub(crate) const NOTIFY_MODES: [&str; 3] = ["all", "waiting", "off"];

/// `shell`, `notify_sound` and `notify_mode` as saved, each falling back to its default (auto, true, all) when missing or invalid.
/// The app itself reads the same keys from settings.json.
fn ui_settings(saved: &Value) -> (&str, bool, &str) {
    let shell = saved["shell"].as_str().filter(|s| SHELLS.contains(s)).unwrap_or("auto");
    let mode = saved["notify_mode"].as_str().filter(|m| NOTIFY_MODES.contains(m)).unwrap_or("all");
    (shell, saved["notify_sound"] != json!(false), mode)
}

/// The error of a request without a required parameter.
pub(crate) fn missing(name: &str) -> anyhow::Error {
    axelstra_core::coded!("rpc.missing_param", "missing {name}", name = name)
}

/// The error of a request for a method this daemon does not have (the English text is what older callers look for).
pub(crate) fn unknown_method(method: &str) -> anyhow::Error {
    axelstra_core::coded!("rpc.unknown_method", "unknown method {method}", method = method)
}

/// Copies the keys of a `settings.set` patch that are present into `saved`, rejecting invalid values.
pub(crate) fn apply_ui_settings(saved: &mut Value, p: &Value) -> Result<()> {
    if let Some(s) = p["shell"].as_str() {
        if !SHELLS.contains(&s) {
            return Err(axelstra_core::coded!("settings.bad_shell", "shell must be auto, pwsh, powershell, bash, wsl or cmd"));
        }
        saved["shell"] = json!(s);
    }
    if let Some(b) = p["notify_sound"].as_bool() {
        saved["notify_sound"] = json!(b);
    }
    if let Some(m) = p["notify_mode"].as_str() {
        if !NOTIFY_MODES.contains(&m) {
            return Err(axelstra_core::coded!("settings.bad_notify_mode", "notify_mode must be all, waiting or off"));
        }
        saved["notify_mode"] = json!(m);
    }
    if let Some(patch) = p.get("ui").filter(|v| !v.is_null()) {
        apply_ui_state(saved, patch)?;
    }
    Ok(())
}

const UI_MAX_KEYS: usize = 64;
const UI_MAX_VALUE: usize = 128 * 1024;
const UI_MAX_TOTAL: usize = 512 * 1024;

/// Merges a `settings.set` `ui` patch (`{key: string | null}`) into `saved["ui"]`: the choices the webview used to keep only
/// in its localStorage (pinned worktrees, folded groups, the notification list...), so they survive a changed WebView
/// profile or a reinstall. A null value removes the key. Nothing is applied unless the whole patch is valid.
fn apply_ui_state(saved: &mut Value, patch: &Value) -> Result<()> {
    let patch = patch.as_object().ok_or_else(|| axelstra_core::coded!("settings.ui_not_object", "ui must be an object of strings"))?;
    let mut ui = saved["ui"].as_object().cloned().unwrap_or_default();
    for (k, v) in patch {
        let key_ok =
            k.len() <= 64 && k.starts_with("axelstra.") && k.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
        if !key_ok {
            return Err(axelstra_core::coded!("settings.ui_key_not_allowed", "ui key {k:?} is not allowed", k = k));
        }
        match v {
            Value::Null => {
                ui.remove(k);
            }
            Value::String(s) if s.len() <= UI_MAX_VALUE => {
                ui.insert(k.clone(), v.clone());
            }
            _ => {
                return Err(axelstra_core::coded!(
                    "settings.ui_bad_value",
                    "ui value for {k:?} must be a string of at most {max} bytes",
                    k = k,
                    max = UI_MAX_VALUE
                ))
            }
        }
    }
    let total: usize = ui.values().filter_map(Value::as_str).map(str::len).sum();
    if ui.len() > UI_MAX_KEYS || total > UI_MAX_TOTAL {
        return Err(axelstra_core::coded!(
            "settings.ui_too_large",
            "ui state is too large ({keys} keys, {total} bytes)",
            keys = ui.len(),
            total = total
        ));
    }
    saved["ui"] = Value::Object(ui);
    Ok(())
}

/// The app-wide settings file next to the daemon state (missing or broken = no settings).
fn read_settings(saved: &std::path::Path) -> Value {
    read_json(&saved.with_file_name("settings.json")).ok().flatten().unwrap_or_else(|| json!({}))
}

#[derive(Deserialize, Clone)]
struct Req {
    id: u64,
    method: String,
    #[serde(default)]
    params: Value,
}

/// True for the real `axelstra` executable. A test binary (`roundtrip-<hash>.exe`) or a renamed copy cannot serve
/// `statusline-tee`, so it must never be written into a status line that every session will run.
fn is_axelstra_exe(exe: &std::path::Path) -> bool {
    exe.file_stem().is_some_and(|s| s.eq_ignore_ascii_case("axelstra"))
}

/// The settings file passed to Claude as `--settings`: a hook that records the agent's current conversation id (it
/// changes on /clear), keyed by AXELSTRA_KEY, and a status line command that hands Claude's own numbers (context
/// window, 5 h and weekly limits) to the app and then runs the user's own status line, so nothing they see changes.
/// Without a real `axelstra` exe the status line is left out, and an existing file is not overwritten.
fn hooks_json(exe: Option<&std::path::Path>) -> Value {
    let mut hook = json!({"hooks": {"SessionStart": [{"hooks": [{"type": "command",
        "command": "[ -n \"$AXELSTRA_KEY\" ] || exit 0; mkdir -p \"$AXELSTRA_DIR/sid\" && cat > \"$AXELSTRA_DIR/sid/$AXELSTRA_KEY.json\""}]}]}});
    if let Some(exe) = exe.filter(|e| is_axelstra_exe(e)) {
        hook["statusLine"] = json!({"type": "command", "command": format!("\"{}\" statusline-tee", exe.display())});
    }
    hook
}

fn write_hooks_json(dir: &std::path::Path, exe: Option<&std::path::Path>) {
    let file = dir.join("hooks.json");
    if !exe.is_some_and(is_axelstra_exe) && file.exists() {
        return;
    }
    let _ = std::fs::write(file, hooks_json(exe).to_string());
}

/// Serves until the process dies. Fails if another daemon owns the pipe.
pub async fn serve(pipe: String, token: String) -> Result<()> {
    serve_with_store(pipe, token, crate::projects::store_path()).await
}

/// Same as `serve`, with an explicit projects file (tests must not touch the user's).
pub async fn serve_with_store(pipe: String, token: String, store: std::path::PathBuf) -> Result<()> {
    crate::proto::refuse_real_when_testing(&pipe, store.parent().unwrap_or(std::path::Path::new("")))?;
    origin::adopt_orphans();
    let reg = Arc::new(Registry::new(store, pipe.clone(), token.clone()));
    let _ = reg.me.set(Arc::downgrade(&reg));
    let dir = reg.saved.parent().map(std::path::Path::to_path_buf).unwrap_or_default();
    let _ = std::fs::create_dir_all(&dir);
    write_hooks_json(&dir, std::env::current_exe().ok().as_deref());
    spawn_scheduler(reg.clone());
    spawn_budget_loop(reg.clone());
    spawn_data_watch(reg.clone());
    spawn_session_monitor(reg.clone());
    // worktrees that exist now and carry no mark become manual, before the janitor may look at them
    if let Err(e) = crate::projects::migrate_marks(&reg.store) {
        eprintln!("janitor: mark migration failed: {e:#}");
    }
    // the daemon starting is one of the moments to look for merged worktrees
    reg.janitor.bump();
    spawn_janitor(reg.clone());
    spawn_stall_watchdog(reg.clone());
    // sessions hibernated when the previous daemon stopped stay hibernated; the waker and the policy sleep until an event
    hibernate::adopt(&reg);
    hibernate::spawn_waker(reg.clone());
    hibernate::spawn_policy(reg.clone());
    triggers::spawn_loop(reg.clone());
    spawn_idle_exit(reg.clone());
    accept_loop(&pipe, reg, token).await
}

#[cfg(windows)]
async fn accept_loop(pipe: &str, reg: Arc<Registry>, token: String) -> Result<()> {
    use tokio::net::windows::named_pipe::ServerOptions;
    // every instance gets the same explicit DACL (current user and SYSTEM), and remote clients are refused
    let sec = crate::pipesec::PipeSecurity::current_user()?;
    let make = |first: bool| -> std::io::Result<tokio::net::windows::named_pipe::NamedPipeServer> {
        let mut attrs = sec.attributes();
        unsafe {
            ServerOptions::new()
                .first_pipe_instance(first)
                .reject_remote_clients(true)
                .create_with_security_attributes_raw(pipe, (&mut attrs as *mut windows_sys::Win32::Security::SECURITY_ATTRIBUTES).cast())
        }
    };
    let mut server = make(true)?;
    loop {
        // one failed accept (a client that vanished mid-connect) must not end the daemon: log, back off, go on
        if let Err(e) = server.connect().await {
            eprintln!("accept failed: {e}");
            tokio::time::sleep(Duration::from_millis(100)).await;
            server = next_instance(&make).await;
            continue;
        }
        let conn = server;
        server = next_instance(&make).await;
        let peer = origin::pipe_peer(&conn);
        tokio::spawn(handle(conn, reg.clone(), token.clone(), peer));
    }
}

/// A fresh pipe instance to accept on (same DACL as the first one), retrying with a short backoff while the system
/// refuses to make one.
#[cfg(windows)]
async fn next_instance(
    make: &impl Fn(bool) -> std::io::Result<tokio::net::windows::named_pipe::NamedPipeServer>,
) -> tokio::net::windows::named_pipe::NamedPipeServer {
    loop {
        match make(false) {
            Ok(s) => return s,
            Err(e) => {
                eprintln!("could not create a pipe instance: {e}");
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
        }
    }
}

#[cfg(unix)]
async fn accept_loop(path: &str, reg: Arc<Registry>, token: String) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    if let Some(dir) = std::path::Path::new(path).parent() {
        std::fs::create_dir_all(dir)?;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    // a live daemon owns the socket; a leftover file from a crashed one is replaced
    if std::path::Path::new(path).exists() {
        if std::os::unix::net::UnixStream::connect(path).is_ok() {
            return Err(axelstra_core::coded!("daemon.already_running", "another daemon is already listening on {path}", path = path));
        }
        std::fs::remove_file(path)?;
    }
    let listener = tokio::net::UnixListener::bind(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    loop {
        match listener.accept().await {
            Ok((conn, _)) => {
                let peer = origin::socket_peer(&conn);
                tokio::spawn(handle(conn, reg.clone(), token.clone(), peer));
            }
            Err(e) => {
                eprintln!("accept failed: {e}");
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    }
}

async fn read_frame(rd: &mut (impl AsyncReadExt + Unpin), max: usize) -> std::io::Result<(u8, Vec<u8>)> {
    let mut len = [0u8; 4];
    rd.read_exact(&mut len).await?;
    let len = u32::from_le_bytes(len) as usize;
    if len == 0 || len > max {
        return Err(std::io::ErrorKind::InvalidData.into());
    }
    let mut kind = [0u8; 1];
    rd.read_exact(&mut kind).await?;
    let mut buf = vec![0u8; len - 1];
    rd.read_exact(&mut buf).await?;
    Ok((kind[0], buf))
}

struct ClientGuard(Arc<Registry>);

impl Drop for ClientGuard {
    fn drop(&mut self) {
        self.0.clients.fetch_sub(1, Ordering::SeqCst);
        self.0.pulse.bump();
    }
}

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
/// Before `hello` a peer may send small frames only: nobody unauthenticated gets to make the daemon allocate 16 MiB.
const PREAUTH_MAX_FRAME: usize = 64 << 10;
/// JSON requests parsed but not yet run on one connection; a client that sends faster waits (backpressure).
const CALL_QUEUE: usize = 256;
/// Keystrokes waiting for their session on one connection.
const INPUT_QUEUE: usize = 1024;
/// How long a full session input backlog is waited for before the keys are given up on (and the client told).
const INPUT_PATIENCE: Duration = Duration::from_secs(3);

/// What one connection accumulates over its life; the call worker owns it while a call runs.
#[derive(Default)]
struct ConnState {
    authed: bool,
    /// set by a `hello` that says it comes from the CLI (what agents run); such a connection cannot switch screen control on
    cli: bool,
    /// set by a `hello` that presented a session's capability token: such a connection has role=agent, and is that session
    agent: Option<u32>,
    /// process id of the client end, where the OS tells it: `hello` classifies the connection by it (see `origin`)
    peer: Option<origin::Peer>,
    subs: HashMap<u32, Arc<AtomicBool>>,
}

/// The result of one dispatch, the deferred work it queued and the delay before its reply.
type Dispatched = (Result<Value>, Vec<Box<dyn FnOnce() + Send>>, Duration);

/// Runs `dispatch` for one call. A `lane` call (a slow read) works on a snapshot of the connection state instead of
/// holding its lock, so it neither blocks nor is blocked by the ordered calls of the same connection.
fn run_dispatch(req: &Req, reg: &Arc<Registry>, tx: &Tx, st: &Arc<Mutex<ConnState>>, token: &str, lane: bool) -> Dispatched {
    let mut after: Vec<Box<dyn FnOnce() + Send>> = Vec::new();
    let mut delay = Duration::ZERO;
    let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if lane {
            let (mut subs, mut authed, mut cli, mut agent, peer) = {
                let g = st.lock_recover();
                (HashMap::new(), g.authed, g.cli, g.agent, g.peer)
            };
            dispatch(&mut Ctx {
                req,
                p: &req.params,
                reg,
                tx,
                subs: &mut subs,
                authed: &mut authed,
                cli: &mut cli,
                agent: &mut agent,
                peer,
                token,
                after: &mut after,
                delay: &mut delay,
            })
        } else {
            let mut guard = st.lock_recover();
            let c = &mut *guard;
            dispatch(&mut Ctx {
                req,
                p: &req.params,
                reg,
                tx,
                subs: &mut c.subs,
                authed: &mut c.authed,
                cli: &mut c.cli,
                agent: &mut c.agent,
                peer: c.peer,
                token,
                after: &mut after,
                delay: &mut delay,
            })
        }
    }))
    .unwrap_or_else(|_| {
        Err(axelstra_core::coded!("daemon.internal_error", "internal daemon error in {method} (see crash.log)", method = req.method))
    });
    (res, after, delay)
}

fn internal_error(method: &str) -> anyhow::Error {
    axelstra_core::coded!("daemon.internal_error", "internal daemon error in {method} (see crash.log)", method = method)
}

/// A slow read: bounded by its lane's gate, shared with identical calls in flight, and answered with `{"loading": true}`
/// (or the last project list) when it outlasts `answer_limit`; the work then goes on and a repeat joins it.
async fn run_lane(
    req: &Arc<Req>,
    lane: crate::lanes::Lane,
    reg: &Arc<Registry>,
    tx: &Tx,
    st: &Arc<Mutex<ConnState>>,
    token: &str,
) -> Result<Value> {
    let (req2, reg2, tx2, st2, token2) = (req.clone(), reg.clone(), tx.clone(), st.clone(), token.to_string());
    // The registry and the project generation are part of the key: `flights` is process-wide, so without them a call
    // would join another daemon's run (tests host several), or a run that started before a project/worktree change.
    let key = format!("{:p}|{}|{}|{}", Arc::as_ptr(reg), reg.wt_gen.load(Ordering::SeqCst), req.method, req.params);
    let mut work = tokio::spawn(crate::lanes::single_flight(key, move || async move {
        let _permit = crate::lanes::gate(lane).acquire().await;
        let method = req2.method.clone();
        tokio::task::spawn_blocking(move || run_dispatch(&req2, &reg2, &tx2, &st2, &token2, true).0)
            .await
            .unwrap_or_else(|_| Err(internal_error(&method)))
    }));
    match tokio::time::timeout(crate::lanes::answer_limit(lane), &mut work).await {
        Ok(joined) => joined.unwrap_or_else(|_| Err(internal_error(&req.method))),
        Err(_) => {
            if req.method == "project.list" {
                let last = reg.plist_last.lock_recover().clone();
                if let Some(v) = last {
                    return Ok(v);
                }
                return work.await.unwrap_or_else(|_| Err(internal_error(&req.method)));
            }
            if req.method == "project.logo" {
                // no logo for now: the caller shows its fallback, a repeat joins the run or hits the cache
                return Ok(json!({"data_url": null, "loading": true}));
            }
            Ok(json!({"loading": true}))
        }
    }
}

/// The reply fields of a failed call: `error`, and the additive `code` and `params` of a coded one.
fn error_fields(e: &anyhow::Error) -> Value {
    let mut b = json!({"error": format!("{e:#}")});
    // additive: an old frontend keeps showing `error`, a new one translates `code` with `params`
    if let Some(c) = axelstra_core::msg::find(e) {
        b["code"] = json!(c.code);
        b["params"] = c.params_json();
    }
    b
}

fn reply_body(id: u64, out: Result<Value, Value>) -> Value {
    match out {
        Ok(v) => json!({"id": id, "result": v}),
        Err(mut e) => {
            e["id"] = json!(id);
            e
        }
    }
}

/// Runs one JSON call and queues the reply (and then the deferred work) on `tx`. Ordered calls run on the blocking pool;
/// slow reads go through `run_lane`.
async fn run_call(req: Req, reg: &Arc<Registry>, tx: &Tx, st: &Arc<Mutex<ConnState>>, token: &str) {
    let t0 = std::time::Instant::now();
    let lane = crate::lanes::lane_of(&req.method).filter(|_| st.lock_recover().authed);
    let (req, res, after, delay) = if let Some(l) = lane {
        let req = Arc::new(req);
        let res = run_lane(&req, l, reg, tx, st, token).await;
        (req, res, Vec::new(), Duration::ZERO)
    } else {
        let (reg2, tx2, st2, token2) = (reg.clone(), tx.clone(), st.clone(), token.to_string());
        // dispatch does blocking work (git, process spawns, files): it runs on the blocking pool, so the connection's
        // reader (and with it every terminal's keystrokes) never waits for it. A bug in one handler must not take every
        // terminal down with the daemon.
        let joined = tokio::task::spawn_blocking(move || {
            let (res, after, delay) = run_dispatch(&req, &reg2, &tx2, &st2, &token2, false);
            (req, res, after, delay)
        })
        .await;
        let Ok((req, res, after, delay)) = joined else { return };
        (Arc::new(req), res, after, delay)
    };
    // what an agent connection asked for is kept for the user to review
    let agent = st.lock_recover().agent;
    if let Some(me) = agent {
        reg.audit.record(me, &req.method, &req.params, &res.as_ref().map(|_| ()).map_err(|e| format!("{e:#}")));
    }
    // calls that wait on purpose are not slow
    if !matches!(req.method.as_str(), "ask" | "msg.ask" | "session.wait" | "hello") {
        crate::lag::note(&format!("daemon {}", req.method), t0.elapsed(), 30);
    }
    let body = reply_body(req.id, res.map_err(|e| error_fields(&e)));
    if !delay.is_zero() {
        tokio::time::sleep(delay).await;
    }
    let _ = tx.send(encode(JSON, body.to_string().as_bytes())).await;
    // deferred work (stream start) runs only after the reply is queued
    for f in after {
        f();
    }
    // a request may have changed what sessions show (a message, a hint, a removal): the monitor looks once. A read
    // changed nothing, and with the app listing and polling all day each bump would wake every reaper and monitor.
    if !reads_only(&req.method) {
        reg.pulse.bump();
    }
}

/// Whether `method` only reads (or touches the disk or git, never a session, its messages or the bus), so the
/// background waiters that watch the pulse have nothing new to look at after it.
fn reads_only(method: &str) -> bool {
    let prefix = method.split('.').next().unwrap_or_default();
    matches!(prefix, "fs" | "files" | "git" | "watch" | "open" | "docker" | "ports" | "host" | "env" | "audit" | "pm" | "agents" | "codex")
        || matches!(
            method,
            "session.list"
                | "session.status"
                | "session.screen"
                | "session.resources"
                | "session.usage"
                | "session.pending"
                | "session.resize"
                | "session.subscribe"
                | "session.unsubscribe"
                | "settings.get"
                | "project.list"
                | "project.logo"
                | "project.log"
                | "project.discover"
                | "project.clone_status"
                | "project.env_preview"
                | "worktree.status"
                | "worktree.overview"
                | "dispatch.meta"
                | "dispatch.overlaps"
                | "dispatch.overlap_files"
                | "triggers.get"
                | "triggers.preview"
                | "agent.status"
                | "agent.search"
                | "agent.usage"
                | "agent.worktree_usage"
                | "budget.status"
                | "hooks.status"
                | "orch.limits"
                | "orch.trust_info"
                | "orch.status"
                | "kanban.list"
                | "automation.list"
                | "flow.list"
                | "computer.status"
                | "app.dir"
                | "gh.list"
                | "gh.comments"
                | "gh.repos"
                | "msg.thread"
                | "msg.reply_of"
                | "notes.read"
                | "notes.read_file"
                | "notes.export"
                | "notes.worktree"
                | "ping"
        )
}

/// Writes one chunk of keystrokes; a full backlog (the program is not reading) is waited out for a while rather than
/// dropping keys at once. Returns false when the keys had to be given up on.
fn write_input(s: &Session, data: &[u8]) -> bool {
    let end = std::time::Instant::now() + INPUT_PATIENCE;
    loop {
        match s.write(data) {
            Ok(()) => return true,
            Err(_) if s.try_wait().is_some() => return false,
            Err(_) if std::time::Instant::now() >= end => return false,
            Err(_) => std::thread::sleep(Duration::from_millis(5)),
        }
    }
}

/// Terminal streams currently forwarding output to a client (tests only; a dead client's stream leaves by itself).
#[doc(hidden)]
pub fn live_streams() -> usize {
    session::live_streams()
}

async fn handle(pipe: impl AsyncRead + AsyncWrite + Send + 'static, reg: Arc<Registry>, token: String, peer: Option<origin::Peer>) {
    reg.clients.fetch_add(1, Ordering::SeqCst);
    reg.pulse.bump();
    let _guard = ClientGuard(reg.clone());
    let (mut rd, mut wr) = tokio::io::split(pipe);
    let (tx, mut rx) = mpsc::channel::<Vec<u8>>(OUT_QUEUE);
    tokio::spawn(async move {
        while let Some(f) = rx.recv().await {
            if wr.write_all(&f).await.is_err() {
                break;
            }
        }
        drop(rx);
        // the writer is gone: `rx` drops, every `tx` reports closed, and the streams of this connection (idle or not) end
        session::wake_streams();
    });

    let st = Arc::new(Mutex::new(ConnState { peer, ..Default::default() }));
    let mut authed = false;
    // calls run in order on one worker (a client may pipeline `spawn` then `attach`), but off the reader
    let (call_tx, mut call_rx) = mpsc::channel::<Req>(CALL_QUEUE);
    let worker = {
        let (reg, tx, st, token) = (reg.clone(), tx.clone(), st.clone(), token.clone());
        tokio::spawn(async move {
            while let Some(req) = call_rx.recv().await {
                if crate::lanes::lane_of(&req.method).is_some() {
                    // a slow read does not hold up the calls queued behind it
                    let (reg, tx, st, token) = (reg.clone(), tx.clone(), st.clone(), token.clone());
                    tokio::spawn(async move { run_call(req, &reg, &tx, &st, &token).await });
                } else {
                    run_call(req, &reg, &tx, &st, &token).await;
                }
            }
        })
    };
    // keystrokes have their own ordered worker, so a program that does not read its input delays only this queue
    let (in_tx, mut in_rx) = mpsc::channel::<(u32, Vec<u8>)>(INPUT_QUEUE);
    // the session this connection is, when it is an agent's (id + 1; 0 for the app and the CLI of the user). It is set by
    // the handshake before any input frame is read, and lives outside `st` because a running call holds that lock.
    let who = Arc::new(AtomicU64::new(0));
    let input_worker = {
        let (reg, tx, who) = (reg.clone(), tx.clone(), who.clone());
        tokio::spawn(async move {
            while let Some((id, data)) = in_rx.recv().await {
                let Some(s) = reg.get(id) else {
                    // typing into a hibernated tab wakes it; the keys cannot reach a process that does not exist yet
                    if reg.hib.is_sleeping(id) {
                        hibernate::wake(&reg, id);
                        let ev = json!({"event": "session.input_dropped", "id": id});
                        let _ = tx.send(encode(EVENT, ev.to_string().as_bytes())).await;
                    }
                    continue;
                };
                let me = who.load(Ordering::SeqCst);
                // an agent types only into itself and the sessions it started: keys are how a prompt gets approved
                let allowed = me == 0 || agent_drives(&reg, (me - 1) as u32, id);
                let t0 = std::time::Instant::now();
                let ok = allowed && tokio::task::spawn_blocking(move || write_input(&s, &data)).await.unwrap_or(false);
                crate::lag::note("daemon pty write", t0.elapsed(), 20);
                if !ok {
                    // the client learns that keys were lost instead of typing into the void
                    let ev = json!({"event": "session.input_dropped", "id": id});
                    let _ = tx.send(encode(EVENT, ev.to_string().as_bytes())).await;
                }
            }
        })
    };

    loop {
        // a connection that has not sent a valid `hello` yet must do so quickly, or it holds a task for nothing
        let frame = if authed {
            read_frame(&mut rd, MAX_FRAME).await
        } else {
            match tokio::time::timeout(HANDSHAKE_TIMEOUT, read_frame(&mut rd, PREAUTH_MAX_FRAME)).await {
                Ok(f) => f,
                Err(_) => break,
            }
        };
        let Ok((kind, payload)) = frame else { break };
        match kind {
            JSON => {
                let Ok(req) = serde_json::from_slice::<Req>(&payload) else { break };
                if !authed {
                    if req.method != "hello" {
                        break;
                    }
                    // the handshake runs inline: nothing else on this connection is trusted until it is done
                    run_call(req, &reg, &tx, &st, &token).await;
                    if !st.lock_recover().authed {
                        break;
                    }
                    if let Some(me) = st.lock_recover().agent {
                        who.store(u64::from(me) + 1, Ordering::SeqCst);
                    }
                    authed = true;
                } else if call_tx.send(req).await.is_err() {
                    break;
                }
            }
            INPUT if authed => {
                if let Some((id, data)) = split_session(&payload) {
                    if in_tx.send((id, data.to_vec())).await.is_err() {
                        break;
                    }
                }
            }
            _ => break,
        }
    }
    // client gone: stop forwarders once the queued calls are done (a call may still add one); sessions keep running
    drop(call_tx);
    drop(in_tx);
    tokio::spawn(async move {
        let _ = worker.await;
        let _ = input_worker.await;
        for stop in st.lock_recover().subs.values() {
            stop.store(true, Ordering::SeqCst);
        }
        crate::watcher::wake_stopped();
        session::wake_streams();
    });
}

/// Lets a watching client hear when the state of any session changed (started, exited, went quiet, escalated, got a
/// message...) and when the worktrees changed behind its back, so the UI does not poll. The events come from
/// `spawn_session_monitor` and `projects_changed`; the client lists sessions itself when it connects.
fn ensure_session_notifier(reg: &Arc<Registry>, tx: &Tx, subs: &mut HashMap<u32, Arc<AtomicBool>>) {
    const KEY: u32 = u32::MAX - 1;
    if subs.contains_key(&KEY) {
        return;
    }
    let stop = Arc::new(AtomicBool::new(false));
    subs.insert(KEY, stop.clone());
    let mut w = reg.watchers.lock_recover();
    // connections that went away since the last event leave the list here too
    w.retain(|(s, _)| !s.load(Ordering::SeqCst));
    w.push((stop, tx.clone()));
}

/// The task file of an agent session, if it has one and the path is still one of ours.
fn note_path(reg: &Registry, id: u32) -> Option<std::path::PathBuf> {
    let note = std::path::PathBuf::from(reg.metas.lock_recover().get(&id)?["note"].as_str()?);
    let root = crate::notes::root(reg.saved.parent()?);
    crate::notes::inside_root(&root, &note).then_some(note)
}

/// What `session.list` answers. Takes sessions, labels, metas in the registry's lock order.
fn session_list(reg: &Registry) -> Vec<Value> {
    // the locks only pick the sessions (cheap views and clones); screen rendering and the waiting heuristics run after
    let picked: Vec<_> = {
        let l = reg.sessions.lock_recover();
        let labels = reg.labels.lock_recover();
        let metas = reg.metas.lock_recover();
        l.iter().map(|(id, s)| (*id, s.clone(), s.view(), labels.get(id).cloned(), metas.get(id).cloned())).collect()
    };
    let stall_ms = reg.with_settings(crate::stall::threshold_ms);
    let stall_min = crate::stall::display_minutes(stall_ms);
    let busy = busy_parents(reg, &picked);
    let mut v: Vec<Value> = picked
        .into_iter()
        .map(|(id, s, view, label, meta)| {
            let running = s.try_wait().is_none();
            let idle = view.idle_ms();
            // waiting is what the agent said (ask, escalate, a hook event), never read off its screen
            let wait = if running { reg.wait_of(id, view.since_input_ms(), idle) } else { None };
            let waiting = wait.is_some();
            let awaiting = if running { reg.awaiting_of(id, view.since_input_ms(), idle) } else { None };
            let facts = stall_facts(reg, id, running, idle, view.since_input_ms(), meta.as_ref(), &busy);
            let stalled = crate::stall::is_stalled(&facts, stall_ms);
            let stall = stalled.then(|| {
                let line = crate::stall::last_line(&view.screen_tail(12));
                let task = label.clone().unwrap_or_else(|| format!("session {id}"));
                let c = axelstra_core::msg::Coded::new(
                    "agent.stalled",
                    format!("{task} has been stalled for {stall_min} min without reporting. Last line: {line}"),
                    &[("task", task.clone()), ("min", stall_min.to_string()), ("line", line)],
                );
                json!({"minutes": stall_min, "code": c.code, "params": c.params_json(), "text": c.to_string()})
            });
            json!({"id": id, "running": running, "idle_ms": idle, "waiting": waiting, "wait": wait.map(|w| json!({"kind": w.kind.as_str(), "text": w.text})), "awaiting": awaiting.map(|a| json!({"reason": a.reason, "since": a.since, "until": a.until})), "stalled": stalled, "stall": stall, "escalated": reg.bus.is_escalated(id), "unread": reg.bus.unread(id), "finished": reg.bus.finished(id).is_some(), "label": label, "meta": meta})
        })
        .collect();
    v.sort_by_key(|x| x["id"].as_u64());
    hibernate::decorate(reg, &mut v);
    v
}

/// One JSON call as the handlers see it: the request, the registry and the state of the connection it came in on.
struct Ctx<'a> {
    req: &'a Req,
    p: &'a Value,
    reg: &'a Arc<Registry>,
    tx: &'a Tx,
    subs: &'a mut HashMap<u32, Arc<AtomicBool>>,
    authed: &'a mut bool,
    cli: &'a mut bool,
    agent: &'a mut Option<u32>,
    peer: Option<origin::Peer>,
    token: &'a str,
    after: &'a mut Vec<Box<dyn FnOnce() + Send>>,
    /// how long to hold the reply back (a refused `hello`)
    delay: &'a mut Duration,
}

/// Whether `method` changes projects or worktrees, which makes the cached project list stale.
fn changes_projects(method: &str) -> bool {
    (method.starts_with("worktree.") && !matches!(method, "worktree.status" | "worktree.overview"))
        || method == "integrate.undo"
        || (method.starts_with("project.")
            && !matches!(
                method,
                "project.list"
                    | "project.logo"
                    | "project.log"
                    | "project.discover"
                    | "project.clone"
                    | "project.clone_status"
                    | "project.clone_cancel"
            ))
}

/// Drops the cached project list and bumps the generation a `project.list` in flight checks before caching its answer.
fn forget_project_list(reg: &Registry) {
    *reg.plist.lock_recover() = None;
    crate::overview::invalidate();
    reg.wt_gen.fetch_add(1, Ordering::SeqCst);
}

fn dispatch(c: &mut Ctx) -> Result<Value> {
    // Forgotten before the call and again after it: a `project.list` that started in between (the UI polls it) would
    // otherwise cache the state from before the change and serve it for the next seconds.
    let (reg, mutating) = (c.reg.clone(), changes_projects(&c.req.method));
    if mutating {
        forget_project_list(&reg);
    }
    let out = dispatch_call(c);
    if mutating {
        forget_project_list(&reg);
        // a change made from the CLI or a flow shows up in the app without it polling (once the reply is queued)
        let r = reg.clone();
        c.after.push(Box::new(move || r.broadcast("projects.changed")));
    }
    out
}

fn dispatch_call(c: &mut Ctx) -> Result<Value> {
    let (req, p) = (c.req, c.p);
    if c.agent.is_some() && agent_forbidden(&req.method, p) {
        return Err(axelstra_core::coded!(
            "rpc.agent_forbidden",
            "{method} is not allowed to agents: it is done by the user in the app",
            method = req.method
        ));
    }
    // an agent reads its own inbox and writes only its own task file: its credential names the session
    if matches!(req.method.as_str(), "msg.inbox" | "notes.append") {
        require_own(*c.agent, p["session"].as_u64(), &req.method)?;
    }
    if sources::handles(&req.method) {
        return sources::call(c);
    }
    match req.method.split('.').next().unwrap_or_default() {
        "session" => session::call(c),
        "worktree" | "project" | "pm" | "janitor" | "dispatch" | "ports" | "integrate" => worktrees::call(c),
        "fs" | "files" | "watch" | "open" => files::call(c),
        "git" | "gh" => vcs::call(c),
        "triggers" => triggers::call(c),
        "docker" => containers::call(c),
        "orch" | "kanban" | "automation" => orchestration::call(c),
        "msg" | "notes" => messaging::call(c),
        "agent" | "agents" | "codex" | "computer" | "settings" | "flow" | "app" | "hooks" | "ping" | "audit" | "notices" | "daemon"
        | "hello" | "backup" | "env" | "budget" | "workspace" | "host" | "debug" => misc::call(c),
        _ => Err(unknown_method(&req.method)),
    }
}

#[cfg(test)]
mod project_main_tests {
    use super::*;

    #[test]
    fn the_project_main_is_the_live_agent_marked_main_not_whoever_sits_in_the_checkout() {
        let dir = std::env::temp_dir().join(format!("axl-reg-main-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let reg = Arc::new(Registry::new(dir.join("projects.json"), "p".into(), "t".into()));
        let (prog, args): (&str, Vec<String>) =
            if cfg!(windows) { ("cmd", vec!["/c".into(), "ping -n 30 127.0.0.1 >nul".into()]) } else { ("sleep", vec!["30".into()]) };
        let sp = Spawn { program: prog.into(), args, cwd: None, env: vec![], cols: 80, rows: 24 };
        let checkout = dir.to_string_lossy().to_string();
        // an agent that happens to sit in the checkout but is not the main, then the main (in another folder)
        let sitter = reg.spawn(&sp, None, Some(json!({"kind": "agent", "project": "p", "worktree": checkout}))).unwrap();
        let main = reg.spawn(&sp, None, Some(json!({"kind": "agent", "project": "p", "main": true, "worktree": "/elsewhere"}))).unwrap();
        let other = reg.spawn(&sp, None, Some(json!({"kind": "agent", "project": "q", "main": true}))).unwrap();
        assert_eq!(reg.project_main("p"), Some(main));
        assert_eq!(reg.project_main("q"), Some(other));
        assert_eq!(reg.project_main("nope"), None);
        // once it is gone there is none: the sitter is not promoted
        reg.get(main).unwrap().kill();
        let t = Instant::now();
        while reg.project_main("p").is_some() {
            assert!(t.elapsed() < Duration::from_secs(10), "main still counted live");
            std::thread::sleep(Duration::from_millis(20));
        }
        for id in [sitter, other] {
            reg.get(id).unwrap().kill();
        }
    }

    #[test]
    fn agents_get_a_long_quiet_spell_before_the_reaper_closes_them() {
        assert_eq!(reaper_min_idle_secs(true), 120);
        assert_eq!(reaper_min_idle_secs(false), 3);
    }
}

#[cfg(test)]
mod bypass_tests {
    use super::*;

    #[test]
    fn mode_names_and_settings_files_decide_bypass() {
        for m in ["bypassPermissions", "yolo", "dontAsk", "dangerously-skip"] {
            assert!(mode_is_bypass(m), "{m}");
        }
        for m in ["default", "plan", "acceptEdits", "auto"] {
            assert!(!mode_is_bypass(m), "{m}");
        }
        assert!(settings_default_bypass(&json!({"permissions": {"defaultMode": "bypassPermissions"}})));
        assert!(!settings_default_bypass(&json!({"permissions": {"defaultMode": "plan"}})));
        assert!(!settings_default_bypass(&json!({})));
    }

    #[test]
    fn the_reported_mode_beats_the_start_arguments_and_goes_with_the_session() {
        let dir = std::env::temp_dir().join(format!("axl-bypass-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let wt = dir.join("wt");
        std::fs::create_dir_all(wt.join(".claude")).unwrap();
        let reg = Arc::new(Registry::new(dir.join("projects.json"), "p".into(), "t".into()));
        reg.metas.lock_recover().insert(7, json!({"worktree": wt.to_string_lossy()}));
        reg.metas.lock_recover().insert(8, json!({}));
        // the worktree's own settings make bypass the default
        std::fs::write(wt.join(".claude").join("settings.local.json"), r#"{"permissions":{"defaultMode":"bypassPermissions"}}"#).unwrap();
        assert!(reg.runs_in_bypass(7));
        assert!(!reg.runs_in_bypass(8));
        // a Shift+Tab switch reported by the hook wins both ways
        reg.modes.lock_recover().insert(7, "default".into());
        reg.modes.lock_recover().insert(8, "bypassPermissions".into());
        assert!(!reg.runs_in_bypass(7));
        assert!(reg.runs_in_bypass(8));
        // the start arguments still count when nothing was reported
        reg.specs.lock_recover().insert(9, json!({"args": ["--dangerously-skip-permissions"]}));
        assert!(reg.runs_in_bypass(9));
    }
}

#[cfg(test)]
mod reaper_scheduler_tests {
    use super::*;

    fn long_sleep() -> Spawn {
        let (prog, args): (&str, Vec<String>) =
            if cfg!(windows) { ("cmd", vec!["/c".into(), "ping -n 30 127.0.0.1 >nul".into()]) } else { ("sleep", vec!["30".into()]) };
        Spawn { program: prog.into(), args, cwd: None, env: vec![], cols: 80, rows: 24 }
    }

    fn registry(tag: &str) -> Arc<Registry> {
        let dir = std::env::temp_dir().join(format!("axl-reaper-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        Arc::new(Registry::new(dir.join("projects.json"), "p".into(), "t".into()))
    }

    fn until(what: &str, mut ok: impl FnMut() -> bool) {
        let t = Instant::now();
        while !ok() {
            assert!(t.elapsed() < Duration::from_secs(20), "{what}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn many_workers_share_one_scheduler_thread_that_ends_when_none_is_left() {
        let reg = registry("one");
        let ids: Vec<u32> = (0..5).map(|_| reg.spawn(&long_sleep(), None, Some(json!({"kind": "shell"}))).unwrap()).collect();
        for id in &ids {
            spawn_reaper(reg.clone(), *id, None, false, 20, 10);
        }
        assert_eq!(reg.reapers.threads_started.load(Ordering::SeqCst), 1, "one thread for five workers");
        assert_eq!(reg.reapers.pending(), 5);
        // a worker ending is an event: its reaper reports and the session leaves, with no request in between
        for id in &ids {
            reg.get(*id).unwrap().kill();
        }
        until("workers not retired", || ids.iter().all(|id| reg.get(*id).is_none()));
        until("scheduler kept tasks", || reg.reapers.pending() == 0);
        // with nothing to watch the thread has ended; a new worker starts a new one
        let id = reg.spawn(&long_sleep(), None, Some(json!({"kind": "shell"}))).unwrap();
        until("scheduler thread did not end", || {
            spawn_reaper(reg.clone(), id, None, false, 20, 10);
            reg.reapers.threads_started.load(Ordering::SeqCst) == 2
        });
        reg.get(id).unwrap().kill();
    }

    #[test]
    fn a_quiet_worker_is_closed_at_its_deadline_without_any_event() {
        let reg = registry("deadline");
        let id = reg.spawn(&long_sleep(), None, Some(json!({"kind": "shell"}))).unwrap();
        // the first look sees it as just started (output counts as work); no pulse comes after it: only the deadline
        spawn_reaper(reg.clone(), id, None, true, 3, 10);
        let t = Instant::now();
        until("quiet worker not closed", || reg.get(id).is_none());
        assert!(t.elapsed() >= Duration::from_secs(2), "closed before its quiet spell: {:?}", t.elapsed());
    }

    #[test]
    fn a_worker_that_reported_is_retired_by_the_scheduler_on_the_event() {
        let reg = registry("done");
        let id = reg.spawn(&long_sleep(), None, Some(json!({"kind": "shell"}))).unwrap();
        spawn_reaper(reg.clone(), id, None, false, 20, 10);
        assert!(reg.get(id).is_some());
        reg.bus.send(Some(id), 999, "done", "ok: done", None, None);
        reg.pulse.bump();
        until("reported worker not retired", || reg.get(id).is_none());
        assert_eq!(reg.exited_of(id).map(|e| e.1), Some(true));
    }
}

#[cfg(test)]
mod pulse_tests {
    use super::*;

    #[test]
    fn listing_and_reading_do_not_wake_the_background_waiters_but_changes_do() {
        for m in ["session.list", "session.status", "settings.get", "fs.read", "fs.write", "git.log", "git.merge", "project.list", "ping"] {
            assert!(reads_only(m), "{m}");
        }
        // sessions, messages, the bus and the hints are what the monitor, reapers and done-closers look at
        for m in [
            "session.spawn",
            "session.kill",
            "session.remove",
            "session.rename",
            "session.hint",
            "session.await",
            "msg.send",
            "msg.inbox",
            "msg.reply",
            "notes.append",
            "gh.send_comment",
            "daemon.shutdown",
            "worktree.remove",
            "flow.set",
            "settings.set",
        ] {
            assert!(!reads_only(m), "{m}");
        }
    }
}

#[cfg(test)]
mod settings_tests {
    use super::*;

    #[test]
    fn ui_state_merges_removes_and_survives_other_patches() {
        let mut saved = json!({});
        apply_ui_settings(&mut saved, &json!({"ui": {"axelstra.pins": "[\"a\"]", "axelstra.groupsFolded": "[]"}})).unwrap();
        apply_ui_settings(&mut saved, &json!({"ui": {"axelstra.groupsFolded": null}, "shell": "wsl"})).unwrap();
        assert_eq!(saved["ui"], json!({"axelstra.pins": "[\"a\"]"}));
        apply_ui_settings(&mut saved, &json!({"notify_mode": "off"})).unwrap();
        assert_eq!(saved["ui"], json!({"axelstra.pins": "[\"a\"]"}));
    }

    #[test]
    fn invalid_ui_state_changes_nothing() {
        let mut saved = json!({"ui": {"axelstra.pins": "[]"}});
        for bad in [
            json!({"other.key": "x"}),
            json!({"axelstra.a b": "x"}),
            json!({"axelstra.pins": 5}),
            json!({"axelstra.pins": "ok", "axelstra.big": "x".repeat(UI_MAX_VALUE + 1)}),
        ] {
            assert!(apply_ui_settings(&mut saved, &json!({ "ui": bad })).is_err());
        }
        assert!(apply_ui_settings(&mut saved, &json!({"ui": ["x"]})).is_err());
        assert_eq!(saved["ui"], json!({"axelstra.pins": "[]"}));
        let many: serde_json::Map<String, Value> = (0..=UI_MAX_KEYS).map(|i| (format!("axelstra.k{i}"), json!("v"))).collect();
        assert!(apply_ui_settings(&mut saved, &json!({"ui": many})).is_err());
    }

    #[test]
    fn ui_settings_default_when_missing_or_invalid() {
        assert_eq!(ui_settings(&json!({})), ("auto", true, "all"));
        assert_eq!(ui_settings(&json!({"shell": "zsh", "notify_mode": "x", "notify_sound": "no"})), ("auto", true, "all"));
    }

    #[test]
    fn ui_settings_patch_persists_valid_keys_and_leaves_others() {
        let mut saved = json!({"terse": "short"});
        apply_ui_settings(&mut saved, &json!({"shell": "wsl", "notify_sound": false, "notify_mode": "waiting"})).unwrap();
        assert_eq!(ui_settings(&saved), ("wsl", false, "waiting"));
        assert_eq!(saved["terse"], "short");
        // a patch without these keys changes nothing
        apply_ui_settings(&mut saved, &json!({"terse": "off"})).unwrap();
        assert_eq!(ui_settings(&saved), ("wsl", false, "waiting"));
    }

    #[test]
    fn ui_settings_patch_rejects_invalid_values() {
        let mut saved = json!({});
        assert!(apply_ui_settings(&mut saved, &json!({"shell": "zsh"})).is_err());
        assert!(apply_ui_settings(&mut saved, &json!({"notify_mode": "loud"})).is_err());
        assert_eq!(saved, json!({}));
    }
}

#[cfg(test)]
mod registry_tests {
    use super::*;

    #[test]
    fn is_within_compares_whole_path_components() {
        assert!(is_within("c:/a/wt", "c:/a/wt"));
        assert!(is_within("c:/a/wt/sub/dir", "c:/a/wt"));
        assert!(!is_within("c:/a/wt-2", "c:/a/wt"));
        assert!(!is_within("c:/a/wtx/sub", "c:/a/wt"));
    }

    #[test]
    fn the_janitor_timer_is_the_next_moment_a_worktree_stops_being_touched_or_the_remote_deadline_when_one_waits() {
        let t = JanitorTiming { quiet: Duration::from_secs(600), gap: Duration::ZERO, slow: Duration::from_secs(1800) };
        let ms = |n| Duration::from_millis(n);
        // a daemon older than the quiet spell and no recent activity: nothing waits, so no timer at all
        assert_eq!(janitor_wake(&t, Duration::from_secs(700), std::iter::empty(), false), None);
        // a worktree waits for its PR to be merged on GitHub: the one remote deadline
        assert_eq!(janitor_wake(&t, Duration::from_secs(700), std::iter::empty(), true), Some(t.slow));
        // a fresh daemon: when it stops being too new to know
        assert_eq!(janitor_wake(&t, Duration::from_secs(100), std::iter::empty(), false), Some(Duration::from_secs(500) + ms(20)));
        // the session that left last decides, the older ones do not
        let left = [Duration::from_secs(550), Duration::from_secs(10), Duration::from_secs(900)];
        assert_eq!(janitor_wake(&t, Duration::from_secs(700), left.into_iter(), true), Some(Duration::from_secs(50) + ms(20)));
        // a remote deadline shorter than the spell wins
        let short = JanitorTiming { slow: Duration::from_secs(30), ..t };
        assert_eq!(janitor_wake(&short, Duration::from_secs(1), std::iter::empty(), true), Some(Duration::from_secs(30)));
        assert_eq!(janitor_wake(&short, Duration::from_secs(1), std::iter::empty(), false), Some(Duration::from_secs(599) + ms(20)));
    }

    /// A worktree made by hand whose branch is merged goes once it has been quiet for the quiet spell, with no event at
    /// all in between: the janitor's own timer looks again.
    #[test]
    fn the_janitor_looks_again_after_the_quiet_spell_without_any_event() {
        let root = std::env::temp_dir().join(format!("axl-reg-jtimer-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let repo = root.join("proj");
        std::fs::create_dir_all(&repo).unwrap();
        let g = |dir: &std::path::Path, args: &[&str]| assert!(axelstra_core::git::run(dir, args).unwrap().ok, "git {args:?}");
        let commit = ["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "--allow-empty", "-m", "x"];
        g(&repo, &["init", "-q", "-b", "main"]);
        g(&repo, &commit);
        let store = root.join("projects.json");
        let p = crate::projects::add(&store, &repo.to_string_lossy()).unwrap();
        let wt = crate::projects::worktree_create(&p, "mine").unwrap();
        g(std::path::Path::new(&wt.path), &commit);
        g(&repo, &["merge", "-q", "--ff-only", "ax/mine"]);
        crate::projects::set_manual(&store, &p.id, &wt.path).unwrap();
        // the registry (and so the daemon's age) starts after the setup, with a short quiet spell
        let mut r = Registry::new(store, "p".into(), "t".into());
        r.timing = JanitorTiming { quiet: Duration::from_secs(3), gap: Duration::ZERO, slow: Duration::from_secs(3600) };
        let reg = Arc::new(r);
        reg.janitor.bump();
        spawn_janitor(reg.clone());
        std::thread::sleep(Duration::from_millis(1000));
        assert!(std::path::Path::new(&wt.path).exists(), "removed while the daemon was still too new to know");
        let t = Instant::now();
        while std::path::Path::new(&wt.path).exists() {
            assert!(t.elapsed() < Duration::from_secs(30), "the janitor never looked again");
            std::thread::sleep(Duration::from_millis(100));
        }
        reg.closing.store(true, Ordering::SeqCst);
        axelstra_core::git::purge_trash(std::path::Path::new(&wt.path).parent().unwrap());
        let _ = axelstra_core::git::remove_tree(&root);
    }

    /// A burst of saves is written once at the end of the spell, and the backup copies are not shifted each time.
    #[test]
    fn saves_in_a_burst_are_debounced_and_do_not_rotate_the_backups() {
        let dir = std::env::temp_dir().join(format!("axl-reg-save-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let reg = Arc::new(Registry::new(dir.join("projects.json"), "p".into(), "t".into()));
        let _ = reg.me.set(Arc::downgrade(&reg));
        let rec =
            |key: &str| json!({"program": "x", "cwd": dir.to_string_lossy(), "meta": {"worktree": dir.to_string_lossy(), "key": key}});
        let saved = || -> Vec<Value> {
            let text = std::fs::read_to_string(dir.join("sessions.json")).unwrap_or_default();
            serde_json::from_str::<Value>(&text).map(|v| v["data"].as_array().cloned().unwrap_or_default()).unwrap_or_default()
        };
        // the first save after a quiet spell is immediate
        reg.specs.lock_recover().insert(1, rec("k1"));
        reg.persist();
        assert_eq!(saved().len(), 1);
        // the next ones come within the spell: they wait, and become one write
        for i in 2..=6 {
            reg.specs.lock_recover().insert(i, rec(&format!("k{i}")));
            reg.persist();
        }
        assert_eq!(saved().len(), 1, "saved before the end of the spell");
        let t = Instant::now();
        while saved().len() < 6 {
            assert!(t.elapsed() < Duration::from_secs(10), "the last save never came");
            std::thread::sleep(Duration::from_millis(20));
        }
        // two writes happened in a row, and neither shifted a backup over the other
        assert!(!dir.join("sessions.json.1").exists(), "a backup of a few seconds ago");
        // an immediate save (shutdown) is written whatever the spell
        reg.specs.lock_recover().insert(7, rec("k7"));
        reg.persist_now();
        assert_eq!(saved().len(), 7);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Writes that race keep the newest snapshot on disk.
    #[test]
    fn racing_saves_leave_the_newest_state_on_disk() {
        let reg = temp_registry("racesave");
        let dir = reg.saved.parent().unwrap().to_path_buf();
        let rec =
            |key: &str| json!({"program": "x", "cwd": dir.to_string_lossy(), "meta": {"worktree": dir.to_string_lossy(), "key": key}});
        let mut hs = vec![];
        for t in 0..4 {
            let (r, rec) = (reg.clone(), rec(&format!("k{t}")));
            hs.push(std::thread::spawn(move || {
                r.specs.lock_recover().insert(t, rec);
                r.persist_now();
            }));
        }
        for h in hs {
            h.join().unwrap();
        }
        reg.persist_now();
        let text = std::fs::read_to_string(dir.join("sessions.json")).unwrap();
        assert_eq!(serde_json::from_str::<Value>(&text).unwrap()["data"].as_array().unwrap().len(), 4);
    }

    /// Settings are read from the disk once, and again only after something says the file changed.
    #[test]
    fn settings_come_from_memory_until_they_are_marked_changed() {
        let reg = temp_registry("settingscache");
        let file = reg.saved.with_file_name("settings.json");
        write_json(&file, &json!({"terse": "short"})).unwrap();
        assert_eq!(reg.settings()["terse"], "short");
        // the file changes behind the cache's back: still the old value
        write_json(&file, &json!({"terse": "tiny"})).unwrap();
        assert_eq!(reg.settings()["terse"], "short");
        assert_eq!(reg.with_settings(|s| s["terse"].clone()), "short");
        reg.settings_changed();
        assert_eq!(reg.settings()["terse"], "tiny");
        // a missing file reads as no settings
        std::fs::remove_file(&file).unwrap();
        reg.settings_changed();
        assert_eq!(reg.settings(), json!({}));
    }

    fn temp_registry(tag: &str) -> Arc<Registry> {
        let dir = std::env::temp_dir().join(format!("axl-reg-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        Arc::new(Registry::new(dir.join("projects.json"), "p".into(), "t".into()))
    }

    #[test]
    fn corrupt_sessions_file_is_kept_aside_and_not_overwritten() {
        let dir = std::env::temp_dir().join(format!("axl-reg-bad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("sessions.json"), "[{ broken").unwrap();
        let reg = Registry::new(dir.join("projects.json"), "p".into(), "t".into());
        assert!(reg.pending.lock_recover().is_empty());
        reg.persist();
        let bad = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|p| p.file_name().unwrap().to_string_lossy().starts_with("sessions.json.bad."))
            .expect("corrupt file kept as .bad.<ts>");
        assert_eq!(std::fs::read_to_string(bad).unwrap(), "[{ broken");
        let saved: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("sessions.json")).unwrap()).unwrap();
        assert_eq!(saved, json!({"v": 1, "data": []}));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sessions_of_deleted_worktrees_are_pruned_at_start_after_a_backup() {
        let dir = std::env::temp_dir().join(format!("axl-reg-prune-{}", std::process::id()));
        let alive = dir.join("wt-alive");
        std::fs::create_dir_all(&alive).unwrap();
        let rec = |w: &std::path::Path| json!({"program": "x", "cwd": w.to_string_lossy(), "meta": {"worktree": w.to_string_lossy()}});
        let gone = dir.join("wt-gone");
        write_json(&dir.join("sessions.json"), &vec![rec(&alive), rec(&gone), rec(&gone)]).unwrap();
        let reg = Registry::new(dir.join("projects.json"), "p".into(), "t".into());
        assert_eq!(reg.pending.lock_recover().len(), 1);
        let saved: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("sessions.json")).unwrap()).unwrap();
        assert_eq!(saved["data"].as_array().unwrap().len(), 1);
        let kept = std::fs::read_dir(dir.join("backups")).unwrap().count();
        assert_eq!(kept, 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_persist_drops_records_of_deleted_worktrees_with_their_sid_files() {
        let dir = std::env::temp_dir().join(format!("axl-reg-persist-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let alive = dir.join("wt-alive");
        std::fs::create_dir_all(&alive).unwrap();
        std::fs::create_dir_all(dir.join("sid")).unwrap();
        let gone = dir.join("wt-gone");
        let rec = |w: &std::path::Path, key: &str| json!({"program": "x", "cwd": w.to_string_lossy(), "meta": {"worktree": w.to_string_lossy(), "key": key}});
        std::fs::write(dir.join("sid").join("aaa.json"), "{}").unwrap();
        std::fs::write(dir.join("sid").join("bbb.json"), "{}").unwrap();
        let reg = Registry::new(dir.join("projects.json"), "p".into(), "t".into());
        *reg.pending.lock_recover() = vec![rec(&alive, "aaa"), rec(&gone, "bbb")];
        reg.persist();
        assert_eq!(reg.pending.lock_recover().len(), 1);
        let saved: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("sessions.json")).unwrap()).unwrap();
        assert_eq!(saved["data"].as_array().unwrap().len(), 1);
        assert!(dir.join("sid").join("aaa.json").exists());
        assert!(!dir.join("sid").join("bbb.json").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn registry_survives_a_poisoned_lock() {
        let reg = temp_registry("poison");
        let r = reg.clone();
        let _ = std::thread::spawn(move || {
            let _g = r.metas.lock().unwrap();
            panic!("handler bug");
        })
        .join();
        assert!(reg.metas.is_poisoned());
        assert!(session_list(&reg).is_empty());
        save_histories(&reg);
    }

    /// `session.list` and `daemon.shutdown` both need sessions and metas: with one lock order they never wait on each other.
    #[test]
    fn lock_order_paths_do_not_deadlock() {
        let reg = temp_registry("order");
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        for i in 0..4 {
            let (r, d) = (reg.clone(), done_tx.clone());
            std::thread::spawn(move || {
                for _ in 0..300 {
                    if i % 2 == 0 {
                        session_list(&r);
                    } else {
                        save_histories(&r);
                        r.persist();
                    }
                }
                let _ = d.send(());
            });
        }
        for _ in 0..4 {
            done_rx.recv_timeout(Duration::from_secs(30)).expect("deadlock between registry locks");
        }
    }
}

#[cfg(test)]
mod hooks_json_tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn only_the_real_axelstra_exe_becomes_the_status_line() {
        let status = |p: &str| hooks_json(Some(Path::new(p)))["statusLine"]["command"].as_str().map(String::from);
        if cfg!(windows) {
            assert_eq!(status(r"D:\x\axelstra.exe").as_deref(), Some(r#""D:\x\axelstra.exe" statusline-tee"#));
        }
        assert_eq!(status("/usr/bin/axelstra").as_deref(), Some(r#""/usr/bin/axelstra" statusline-tee"#));
        assert_eq!(status(r"C:\t\deps\roundtrip-d0504a754dee3ab3.exe"), None);
        assert_eq!(status(r"C:\t\deps\axelstra_daemon-1a2b.exe"), None);
        assert!(hooks_json(None)["statusLine"].is_null());
        assert!(hooks_json(None)["hooks"]["SessionStart"].is_array());
    }

    #[test]
    fn a_test_binary_does_not_clobber_the_hooks_file() {
        let dir = std::env::temp_dir().join(format!("axl-hj-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (real, test_exe) =
            if cfg!(windows) { (r"C:\x\axelstra.exe", r"C:\x\deps\roundtrip-1.exe") } else { ("/x/axelstra", "/x/deps/roundtrip-1") };
        write_hooks_json(&dir, Some(Path::new(real)));
        let good = std::fs::read_to_string(dir.join("hooks.json")).unwrap();
        assert!(good.contains("statusline-tee"));
        write_hooks_json(&dir, Some(Path::new(test_exe)));
        assert_eq!(std::fs::read_to_string(dir.join("hooks.json")).unwrap(), good);
        // first run from a non-axelstra exe still gets the session hook, just no status line
        let fresh = dir.join("fresh");
        std::fs::create_dir_all(&fresh).unwrap();
        write_hooks_json(&fresh, Some(Path::new(test_exe)));
        let t = std::fs::read_to_string(fresh.join("hooks.json")).unwrap();
        assert!(t.contains("SessionStart") && !t.contains("statusLine"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
