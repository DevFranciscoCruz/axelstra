#![cfg_attr(all(not(debug_assertions), windows), windows_subsystem = "windows")]

//! Thin UI host: every call is forwarded to the axelstra daemon over its pipe.
//! The daemon owns terminals and orchestration, so closing this window loses nothing.

use anyhow::{Context, Result};
use axelstra_daemon::client::{Client, Frame};
use axelstra_daemon::proto;
use serde_json::{json, Value};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::Manager;

#[derive(Default)]
struct State {
    /// request/response calls
    control: Mutex<Option<Client>>,
    /// keystrokes only, so typing never waits behind a slow control call
    input: Mutex<Option<Client>>,
    /// slow background polling (git status of every worktree, gh, transcript scans): its own connection,
    /// so creating a worktree or listing sessions never waits behind it
    bg: Mutex<Option<Client>>,
    /// per session id, one stop flag per attach token (a remounted view can attach again before the old one detaches)
    attachments: Mutex<std::collections::HashMap<u32, std::collections::HashMap<u64, Arc<AtomicBool>>>>,
    /// write side of the change-event connection (see `watch_start`)
    watch: Mutex<Option<Client>>,
    /// an update already downloaded and verified, waiting for a restart (or for the app to be closed)
    pending: Mutex<Option<(tauri_plugin_updater::Update, Vec<u8>)>>,
}

#[cfg(all(windows, target_env = "msvc", debug_assertions))]
mod debugcrt;
mod gpu;
mod stats;
mod voice;

fn connect() -> Result<Client> {
    let token = proto::load_or_create_token()?;
    let pipe = proto::pipe_name();
    if let Ok(mut c) = Client::connect(&pipe, &token) {
        if c.proto >= proto::PROTO {
            return Ok(c);
        }
        // A daemon from before an update keeps running (so terminals and agents survive it). One that
        // is too old for this app is replaced; its terminals come back through session restore.
        let _ = c.call("daemon.shutdown", json!({}));
        drop(c);
        for _ in 0..40 {
            std::thread::sleep(Duration::from_millis(100));
            if Client::connect(&pipe, &token).is_err() {
                break;
            }
        }
    }
    start_daemon()?;
    for _ in 0..40 {
        std::thread::sleep(Duration::from_millis(100));
        if let Ok(c) = Client::connect(&pipe, &token) {
            return Ok(c);
        }
    }
    anyhow::bail!("daemon did not start")
}

/// Starts `axelstra daemon` detached, so it outlives this window.
fn start_daemon() -> Result<()> {
    let name = if cfg!(windows) { "axelstra.exe" } else { "axelstra" };
    let beside = std::env::current_exe()?.with_file_name(name);
    let exe = if beside.exists() { beside } else { "axelstra".into() };
    let mut cmd = Command::new(exe);
    cmd.arg("daemon").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(DETACHED_PROCESS | CREATE_NO_WINDOW);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // own process group: closing the app (or its terminal) does not signal the daemon
        cmd.process_group(0);
    }
    cmd.spawn().context("start axelstra daemon (build it with `cargo build -p axelstra`)")?;
    Ok(())
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
        .map(|v| v[key].clone())
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
    "dispatch.meta",
    "flow.list",
    "fs.create",
    "fs.delete",
    "fs.grep",
    "fs.list",
    "fs.read",
    "fs.rename",
    "fs.search",
    "fs.stat",
    "fs.write",
    "gh.list",
    "git.changes",
    "git.commit",
    "git.diff",
    "git.log",
    "git.pr",
    "git.push",
    "git.show",
    "git.suggest",
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
    "git.merge",
    "project.env_preview",
    "worktree.rename",
    "dispatch.overlaps",
    "notes.read",
    "notes.export",
    "worktree.overview",
    "worktree.sweep",
    "notes.read_file",
    "notes.worktree",
    "worktree.set_parent",
    "kanban.delete",
    "kanban.list",
    "kanban.move",
    "kanban.save",
    "open.editor",
    "orch.limits",
    "orch.retry",
    "orch.start",
    "orch.status",
    "ping",
    "pm.detect",
    "ports.list",
    "project.add",
    "project.list",
    "project.logo",
    "project.remove",
    "project.setup",
    "session.kill",
    "session.list",
    "session.pending",
    "session.pending_remove",
    "session.remove",
    "session.rename",
    "session.resize",
    "session.resources",
    "session.spawn",
    "settings.get",
    "settings.set",
    "worktree.create",
    "worktree.from_pr",
    "worktree.remove",
    "worktree.status",
];

fn check_allowed(method: &str) -> Result<(), String> {
    if ALLOWED_METHODS.contains(&method) {
        Ok(())
    } else {
        Err(format!("método do daemon não permitido: {method}"))
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
fn check_params(method: &str, params: &Value) -> Result<(), String> {
    match method {
        "session.spawn" => {
            let program = params["program"].as_str().unwrap_or_default();
            if !is_shell(program) {
                return Err(format!("programa não permitido: {program}"));
            }
            if let Some(env) = params["env"].as_object() {
                if let Some(k) = env.keys().find(|k| !k.starts_with("AXELSTRA_")) {
                    return Err(format!("variável de ambiente não permitida: {k}"));
                }
            }
            Ok(())
        }
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
                Err(format!("caminho não permitido: {path}"))
            }
        }
        _ => Ok(()),
    }
}

fn check_call(method: &str, params: &Value) -> Result<(), String> {
    check_allowed(method)?;
    check_params(method, params)
}

/// Only one caller restarts the daemon connection at a time; the others wait for it outside any slot lock.
static RECONNECT: Mutex<()> = Mutex::new(());
/// How long a caller waits for another one's reconnect before giving up.
const RECONNECT_WAIT: Duration = Duration::from_secs(12);

/// Fills an empty `slot` with a live connection. The slot lock is never held while connecting or sleeping, so calls on
/// other slots (and `slot` itself) are not stuck behind a restart that can take seconds.
fn reconnect_into(slot: &Mutex<Option<Client>>) -> Result<()> {
    let deadline = std::time::Instant::now() + RECONNECT_WAIT;
    loop {
        if slot.lock().unwrap().is_some() {
            return Ok(());
        }
        let gate = match RECONNECT.try_lock() {
            Ok(g) => Some(g),
            Err(std::sync::TryLockError::Poisoned(p)) => Some(p.into_inner()),
            Err(std::sync::TryLockError::WouldBlock) => None,
        };
        if let Some(_gate) = gate {
            // another caller may have finished while this one took the gate
            if slot.lock().unwrap().is_some() {
                return Ok(());
            }
            let c = reconnect()?;
            *slot.lock().unwrap() = Some(c);
            return Ok(());
        }
        if std::time::Instant::now() >= deadline {
            anyhow::bail!("o daemon está a reiniciar; tenta outra vez dentro de instantes");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn with_client<T>(slot: &Mutex<Option<Client>>, retry: bool, f: impl Fn(&mut Client) -> Result<T>) -> Result<T> {
    let mut g = slot.lock().unwrap();
    if g.is_none() {
        drop(g);
        reconnect_into(slot)?;
        g = slot.lock().unwrap();
    }
    let Some(c) = g.as_mut() else { anyhow::bail!("sem ligação ao daemon") };
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
                match slot.lock().unwrap().as_mut() {
                    Some(c) => f(c),
                    None => Err(e),
                }
            } else {
                Err(e.context("a ligação ao daemon caiu durante o pedido; confirma o estado antes de repetir"))
            }
        }
    }
}

#[tauri::command]
async fn daemon_call(state: tauri::State<'_, Arc<State>>, method: String, params: Value) -> Result<Value, String> {
    check_call(&method, &params)?;
    let st = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        with_client(&st.control, read_only(&method), |c| c.call(&method, params.clone())).map_err(|e| format!("{e:#}"))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn daemon_call_bg(state: tauri::State<'_, Arc<State>>, method: String, params: Value) -> Result<Value, String> {
    check_call(&method, &params)?;
    let st = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        with_client(&st.bg, read_only(&method), |c| c.call(&method, params.clone())).map_err(|e| format!("{e:#}"))
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

/// Keystrokes go to the daemon through one writer thread: in order (a lock race between per-key tasks could swap
/// them), and a stalled daemon never blocks the UI. Bytes queued for the same session are sent as one frame.
static INPUT_TX: std::sync::OnceLock<std::sync::mpsc::Sender<(u32, Vec<u8>)>> = std::sync::OnceLock::new();

fn input_sender(st: &Arc<State>) -> &'static std::sync::mpsc::Sender<(u32, Vec<u8>)> {
    INPUT_TX.get_or_init(|| {
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
                    let _ = with_client(&st.input, true, |c| c.send_input(id, &data));
                    lag_note("keystroke to daemon", t.elapsed());
                }
            }
        });
        tx
    })
}

#[tauri::command]
fn session_write(state: tauri::State<'_, Arc<State>>, id: u32, data: String) -> Result<(), String> {
    input_sender(state.inner()).send((id, data.into_bytes())).map_err(|e| e.to_string())
}

/// Streams a session to the webview: raw bytes for output, a JSON string for events.
#[tauri::command]
async fn session_attach(state: tauri::State<'_, Arc<State>>, id: u32, on_data: Channel<InvokeResponseBody>) -> Result<u64, String> {
    static NEXT_TOKEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let token = NEXT_TOKEN.fetch_add(1, Ordering::SeqCst);
    let stop = Arc::new(AtomicBool::new(false));
    state.attachments.lock().unwrap().entry(id).or_default().insert(token, stop.clone());
    let st = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || -> Result<u64, String> {
        let forget = || {
            if let Some(m) = st.attachments.lock().unwrap().get_mut(&id) {
                m.remove(&token);
            }
        };
        let mut c = connect().map_err(|e| {
            forget();
            format!("{e:#}")
        })?;
        c.call("session.subscribe", json!({"id": id})).map_err(|e| {
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
                    Frame::Event(e) => on_data.send(InvokeResponseBody::Json(e.to_string())),
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
) -> Result<(), String> {
    let st = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
        let c = connect().map_err(|e| format!("{e:#}"))?;
        let mut rd = c.try_clone().map_err(|e| format!("{e:#}"))?;
        *st.watch.lock().unwrap() = Some(c);
        std::thread::spawn(move || {
            while let Ok(f) = rd.read() {
                if let Frame::Event(e) = f {
                    if on_event.send(InvokeResponseBody::Json(e.to_string())).is_err() {
                        break;
                    }
                }
            }
            // the connection dropped (daemon gone) or the channel closed: the UI starts the stream again
            use tauri::Emitter;
            let _ = app.emit("watch-ended", ());
        });
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Watch this worktree (replacing the previous one), or nothing when `worktree` is null.
#[tauri::command]
async fn watch_set(state: tauri::State<'_, Arc<State>>, worktree: Option<String>) -> Result<(), String> {
    let st = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
        let mut g = st.watch.lock().unwrap();
        let c = g.as_mut().ok_or("watch not started")?;
        match worktree {
            Some(w) => c.send("watch.subscribe", json!({"worktree": w})),
            None => c.send("watch.unsubscribe", json!({})),
        }
        .map_err(|e| format!("{e:#}"))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
/// Stops the attach `token` returned by `session_attach`; without a token, every attach of the session.
fn session_detach(state: tauri::State<'_, Arc<State>>, id: u32, token: Option<u64>) {
    let mut g = state.attachments.lock().unwrap();
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
                if let Ok(mut c) = Client::connect(&pipe, &token) {
                    let _ = c.call("daemon.shutdown", json!({}));
                    drop(c);
                    for _ in 0..30 {
                        std::thread::sleep(Duration::from_millis(100));
                        if Client::connect(&pipe, &token).is_err() {
                            break;
                        }
                    }
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
async fn diagnostics(state: tauri::State<'_, Arc<State>>) -> Result<Value, String> {
    let st = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let daemon = with_client(&st.bg, true, |c| Ok((c.proto, c.instance))).ok();
        let lag = proto::data_dir().join("lag.log");
        json!({
            "app_version": env!("CARGO_PKG_VERSION"),
            "app_proto": proto::PROTO,
            "daemon_connected": daemon.is_some(),
            "daemon_proto": daemon.map(|d| d.0),
            "daemon_instance": daemon.map(|d| d.1),
            "data_dir": proto::data_dir().to_string_lossy(),
            "lag_log": lag.to_string_lossy(),
            "lag_log_bytes": std::fs::metadata(&lag).map(|m| m.len()).unwrap_or(0),
            "missing_tools": missing_tools(),
        })
    })
    .await
    .map_err(|e| e.to_string())
}

/// Deletes pasted clipboard images older than a day.
fn clean_pastes() {
    let dir = std::env::temp_dir().join("axelstra-paste");
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let cutoff = Duration::from_secs(24 * 3600);
    for e in rd.flatten() {
        let old = e.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).map(|age| age > cutoff).unwrap_or(false);
        if old && e.path().extension().is_some_and(|x| x == "png") {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

#[tauri::command]
fn default_shell() -> (String, Vec<String>) {
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
/// Images are saved as PNG in the temp dir and returned as a path to type into the terminal.
#[tauri::command]
fn clipboard_read() -> Result<Value, String> {
    let mut cb = arboard::Clipboard::new().map_err(|e| e.to_string())?;
    if let Ok(img) = cb.get_image() {
        let dir = std::env::temp_dir().join("axelstra-paste");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
        let path = dir.join(format!("paste-{ts}.png"));
        let file = std::fs::File::create(&path).map_err(|e| e.to_string())?;
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

#[tauri::command]
fn clipboard_write(text: String) -> Result<(), String> {
    arboard::Clipboard::new().and_then(|mut c| c.set_text(text)).map_err(|e| e.to_string())
}

/// `None` = this build is the latest. Errors become one fixed sentence: the plugin's own
/// error can quote whatever the endpoint answered.
#[tauri::command]
async fn update_check(app: tauri::AppHandle) -> Result<Option<UpdateInfo>, String> {
    use tauri_plugin_updater::UpdaterExt;
    let updater = app.updater().map_err(|_| "updater unavailable".to_string())?;
    match updater.check().await {
        Ok(Some(u)) => Ok(Some(UpdateInfo { version: u.version, notes: u.body })),
        Ok(None) => Ok(None),
        Err(_) => Err("update check failed".into()),
    }
}

/// Downloads and verifies the update in the background and keeps it for later. `expected` is the
/// version the user was told about: if a newer release landed since, nothing is kept.
#[tauri::command]
async fn update_download(app: tauri::AppHandle, state: tauri::State<'_, Arc<State>>, expected: String) -> Result<(), String> {
    use tauri_plugin_updater::UpdaterExt;
    if let Some((u, _)) = state.pending.lock().unwrap().as_ref() {
        if u.version == expected {
            return Ok(());
        }
    }
    let updater = app.updater().map_err(|_| "updater unavailable".to_string())?;
    let update = updater.check().await.map_err(|_| "update check failed".to_string())?.ok_or("already up to date")?;
    if update.version != expected {
        return Err(format!("a newer release ({}) appeared; check again", update.version));
    }
    let bytes = update.download(|_, _| {}, || {}).await.map_err(|_| "download or signature check failed".to_string())?;
    *state.pending.lock().unwrap() = Some((update, bytes));
    Ok(())
}

/// Installs the downloaded update and restarts the window. The installer renames the running daemon
/// instead of killing it, so terminals and agents keep running through the update.
#[tauri::command]
fn update_apply(app: tauri::AppHandle, state: tauri::State<'_, Arc<State>>) -> Result<(), String> {
    let (update, bytes) = state.pending.lock().unwrap().take().ok_or("no update downloaded")?;
    update.restart_after_install(true).install(bytes).map_err(|_| "install failed".to_string())?;
    app.restart();
}

fn main() {
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
            session_write,
            lag_log,
            session_attach,
            session_detach,
            watch_start,
            watch_set,
            default_shell,
            update_check,
            update_download,
            update_apply,
            clipboard_read,
            clipboard_write,
            notify_session,
            app_quit,
            diagnostics,
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
                let pending = state.pending.lock().unwrap().take();
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

    /// Every `call("x.y"` / `callBg("x.y"` in the frontend (an optional `<Type>` after the name is skipped).
    /// A call whose method is not a string literal is returned as `?file`, since the list cannot cover it.
    fn called_methods() -> BTreeSet<String> {
        let mut files = vec![];
        sources(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../src"), &mut files);
        assert!(!files.is_empty(), "frontend sources not found");
        let mut found = BTreeSet::new();
        for f in files {
            let t = std::fs::read_to_string(&f).unwrap();
            let b = t.as_bytes();
            for name in ["call", "callBg"] {
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
        for bad in ["", "../x", "/etc/passwd", "C:\\x", ".git/config", ".GIT\\hooks\\pre-commit", "a/.git/b", "a/../b"] {
            assert!(check_call("fs.write", &json!({"path": bad})).is_err(), "{bad}");
        }
        assert!(check_call("fs.read", &json!({"path": "../x"})).is_ok());
    }

    #[test]
    fn unknown_methods_are_refused() {
        assert!(check_allowed("session.spawn").is_ok());
        assert!(check_allowed("session.pending_clear").is_err());
        assert!(check_allowed("").is_err());
        assert!(check_allowed("flow.set").unwrap_err().contains("não permitido"));
    }
}
