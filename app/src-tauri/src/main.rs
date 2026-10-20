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

fn with_client<T>(slot: &Mutex<Option<Client>>, f: impl Fn(&mut Client) -> Result<T>) -> Result<T> {
    let mut g = slot.lock().unwrap();
    if g.is_none() {
        *g = Some(connect()?);
    }
    match f(g.as_mut().unwrap()) {
        Ok(v) => Ok(v),
        Err(_) => {
            // the connection may be stale (daemon restarted): reconnect once
            *g = Some(connect()?);
            f(g.as_mut().unwrap())
        }
    }
}

#[tauri::command]
async fn daemon_call(state: tauri::State<'_, Arc<State>>, method: String, params: Value) -> Result<Value, String> {
    let st = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        with_client(&st.control, |c| c.call(&method, params.clone())).map_err(|e| format!("{e:#}"))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn daemon_call_bg(state: tauri::State<'_, Arc<State>>, method: String, params: Value) -> Result<Value, String> {
    let st = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        with_client(&st.bg, |c| c.call(&method, params.clone())).map_err(|e| format!("{e:#}"))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn session_write(state: tauri::State<'_, Arc<State>>, id: u32, data: String) -> Result<(), String> {
    let st = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        with_client(&st.input, |c| c.send_input(id, data.as_bytes())).map_err(|e| format!("{e:#}"))
    })
    .await
    .map_err(|e| e.to_string())?
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
fn app_quit(app: tauri::AppHandle) {
    app.exit(0);
}

#[tauri::command]
fn default_shell() -> (String, Vec<String>) {
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
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .manage(Arc::new(State::default()))
        .manage(voice::Voice::default())
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
        .invoke_handler(tauri::generate_handler![daemon_call, daemon_call_bg, session_write, session_attach, session_detach, watch_start, watch_set, default_shell, update_check, update_download, update_apply, clipboard_read, clipboard_write, notify_session, app_quit, stats::keep_awake, stats::mem_usage, voice::voice_status, voice::voice_download, voice::voice_transcribe])
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
