//! Dictation is not available on Linux: the upstream sherpa-onnx linux-x64-static archive contains no libraries, so the
//! engine cannot be linked (see `Cargo.toml`). The frontend hides the microphone button there; these commands exist so
//! the invoke handler is the same on every platform and any stray call gets a clear error.
use tauri::{ipc::Request, AppHandle};

const UNAVAILABLE: &str = "voice dictation is not available on Linux";

#[derive(Default)]
pub struct Voice;

#[tauri::command]
pub fn voice_status(_app: AppHandle) -> Result<bool, String> {
    Err(UNAVAILABLE.into())
}

#[tauri::command]
pub async fn voice_download(_app: AppHandle) -> Result<(), String> {
    Err(UNAVAILABLE.into())
}

#[tauri::command]
pub async fn voice_transcribe(_app: AppHandle, _request: Request<'_>) -> Result<String, String> {
    Err(UNAVAILABLE.into())
}
