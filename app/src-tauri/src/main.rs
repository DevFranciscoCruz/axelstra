#![cfg_attr(all(not(debug_assertions), windows), windows_subsystem = "windows")]

//! Thin UI host: every call is forwarded to the axelstra daemon over its pipe.
//! The daemon owns terminals and orchestration, so closing this window loses nothing.

use anyhow::{Context, Result};
use axelstra_daemon::client::{Client, Frame};
use axelstra_daemon::proto;
use serde_json::{json, Value};
use std::os::windows::process::CommandExt;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::ipc::{Channel, InvokeResponseBody};

#[derive(Default)]
struct State {
    /// request/response calls
    control: Mutex<Option<Client>>,
    /// keystrokes only, so typing never waits behind a slow control call
    input: Mutex<Option<Client>>,
    attachments: Mutex<std::collections::HashMap<u32, Arc<AtomicBool>>>,
}

fn connect() -> Result<Client> {
    let token = proto::load_or_create_token()?;
    let pipe = proto::pipe_name();
    if let Ok(c) = Client::connect(&pipe, &token) {
        return Ok(c);
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
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let beside = std::env::current_exe()?.with_file_name("axelstra.exe");
    let exe = if beside.exists() { beside } else { "axelstra".into() };
    Command::new(exe)
        .arg("daemon")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(DETACHED_PROCESS | CREATE_NO_WINDOW)
        .spawn()
        .context("start axelstra daemon (build it with `cargo build -p axelstra`)")?;
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

#[tauri::command]
fn session_detach(state: tauri::State<'_, Arc<State>>, id: u32) {
    if let Some(stop) = state.attachments.lock().unwrap().remove(&id) {
        stop.store(true, Ordering::SeqCst);
    }
}

fn main() {
    tauri::Builder::default()
        .manage(Arc::new(State::default()))
        .invoke_handler(tauri::generate_handler![daemon_call, session_write, session_attach, session_detach])
        .run(tauri::generate_context!())
        .expect("error while running axelstra");
}
