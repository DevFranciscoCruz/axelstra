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
    attachments: Mutex<std::collections::HashMap<u32, Arc<AtomicBool>>>,
    /// write side of the change-event connection (see `watch_start`)
    watch: Mutex<Option<Client>>,
    /// an update already downloaded and verified, waiting for a restart (or for the app to be closed)
    pending: Mutex<Option<(tauri_plugin_updater::Update, Vec<u8>)>>,
}

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
    let c = connect()?;
    let prev = LAST_INSTANCE.swap(c.instance, Ordering::SeqCst);
    if prev != 0 && c.instance != prev {
        if let Some(app) = APP.get() {
            use tauri::Emitter;
            let _ = app.emit("daemon-restarted", ());
        }
    }
    Ok(c)
}

/// Calls that only read: safe to run twice if the connection broke after the daemon may have received them.
fn read_only(method: &str) -> bool {
    let tail = method.rsplit('.').next().unwrap_or(method);
    matches!(tail, "list" | "status" | "get" | "read" | "screen" | "usage" | "ports" | "pending" | "info" | "stats" | "log" | "inbox" | "thread" | "search" | "logo" | "dir" | "limits" | "diff" | "changes")
}

fn with_client<T>(slot: &Mutex<Option<Client>>, retry: bool, f: impl Fn(&mut Client) -> Result<T>) -> Result<T> {
    let mut g = slot.lock().unwrap();
    if g.is_none() {
        *g = Some(reconnect()?);
    }
    match f(g.as_mut().unwrap()) {
        Ok(v) => Ok(v),
        // the daemon answered with an error: the connection is fine and the call must not run twice
        Err(e) if e.downcast_ref::<axelstra_daemon::client::Remote>().is_some() => Err(e),
        Err(e) => {
            // the connection may be stale (daemon restarted): reconnect. Run the call again only when that
            // cannot repeat a side effect (it never left, or it only reads)
            let never_sent = e.downcast_ref::<axelstra_daemon::client::NotSent>().is_some();
            *g = None;
            *g = Some(reconnect()?);
            if retry || never_sent {
                f(g.as_mut().unwrap())
            } else {
                Err(e.context("a ligação ao daemon caiu durante o pedido; confirma o estado antes de repetir"))
            }
        }
    }
}

#[tauri::command]
async fn daemon_call(state: tauri::State<'_, Arc<State>>, method: String, params: Value) -> Result<Value, String> {
    let st = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        with_client(&st.control, read_only(&method), |c| c.call(&method, params.clone())).map_err(|e| format!("{e:#}"))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn daemon_call_bg(state: tauri::State<'_, Arc<State>>, method: String, params: Value) -> Result<Value, String> {
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
async fn session_attach(
    state: tauri::State<'_, Arc<State>>,
    id: u32,
    on_data: Channel<InvokeResponseBody>,
) -> Result<(), String> {
    let stop = Arc::new(AtomicBool::new(false));
    if let Some(old) = state.attachments.lock().unwrap().insert(id, stop.clone()) {
        old.store(true, Ordering::SeqCst);
    }
    tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
        let mut c = connect().map_err(|e| format!("{e:#}"))?;
        c.call("session.subscribe", json!({"id": id})).map_err(|e| format!("{e:#}"))?;
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
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Starts the one change-event stream of the window: `fs.changed` events arrive as JSON strings.
/// The connection stays open; `watch_set` points it at a worktree.
#[tauri::command]
async fn watch_start(state: tauri::State<'_, Arc<State>>, on_event: Channel<InvokeResponseBody>) -> Result<(), String> {
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
fn session_detach(state: tauri::State<'_, Arc<State>>, id: u32) {
    if let Some(stop) = state.attachments.lock().unwrap().remove(&id) {
        stop.store(true, Ordering::SeqCst);
    }
}

/// What a "new terminal" should run on this OS: the user's login shell on Unix, PowerShell on Windows.
/// Desktop toast whose click brings the window forward and tells the UI which session it came from.
/// Returns false where the OS toast has no click callback (the UI then uses the plain notification plugin).
#[tauri::command]
fn notify_session(app: tauri::AppHandle, title: String, body: String, worktree: String, session: u32) -> bool {
    // the taskbar button flashes until the window is brought back, even when the toast was missed
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.request_user_attention(Some(tauri::UserAttentionType::Informational));
    }
    #[cfg(windows)]
    {
        use tauri::Emitter;
        // Windows hides toasts (and their sound) while a game or presentation is fullscreen; a plain system
        // beep still gets through, so the toast stays silent and this one is the audible part.
        unsafe { windows_sys::Win32::System::Diagnostics::Debug::MessageBeep(windows_sys::Win32::UI::WindowsAndMessaging::MB_ICONASTERISK) };
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
        let _ = (app, title, body, worktree, session);
        false
    }
}

/// The UI confirmed the close: exit for real (the exit hook still installs a downloaded update).
#[tauri::command]
fn app_quit(app: tauri::AppHandle, keep: Option<bool>) {
    // by default nothing is left running: the daemon ends its terminals and agents and exits too. They come
    // back through session restore the next time the app opens. `keep` leaves them (and automations) running.
    if keep != Some(true) {
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
    }
    app.exit(0);
}

/// Which of the tools the app leans on are not on PATH (git, claude, codex, gh) plus other agent CLIs worth offering (opencode, gemini, aider).
#[tauri::command]
fn missing_tools() -> Vec<String> {
    let dirs: Vec<std::path::PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    let exts: Vec<String> = if cfg!(windows) {
        std::env::var("PATHEXT").unwrap_or_else(|_| ".EXE;.CMD;.BAT".into()).split(';').map(|e| e.to_lowercase()).collect()
    } else {
        vec![String::new()]
    };
    let has = |name: &str| dirs.iter().any(|d| exts.iter().any(|e| d.join(format!("{name}{e}")).is_file()));
    ["git", "claude", "codex", "gh", "opencode", "gemini", "aider"].into_iter().filter(|t| !has(t)).map(String::from).collect()
}

#[tauri::command]
fn default_shell()-> (String, Vec<String>) {
    if cfg!(windows) {
        ("powershell".into(), vec!["-NoLogo".into()])
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
    let update = updater
        .check()
        .await
        .map_err(|_| "update check failed".to_string())?
        .ok_or("already up to date")?;
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
        .invoke_handler(tauri::generate_handler![missing_tools, daemon_call, daemon_call_bg, session_write, lag_log, session_attach, session_detach, watch_start, watch_set, default_shell, update_check, update_download, update_apply, clipboard_read, clipboard_write, notify_session, app_quit, stats::keep_awake, stats::mem_usage, voice::voice_status, voice::voice_download, voice::voice_transcribe])
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
