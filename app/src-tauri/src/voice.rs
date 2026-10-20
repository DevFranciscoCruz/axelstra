//! Local speech to text: Whisper (base, int8) through sherpa-onnx. Audio never leaves the machine.
//! The webview records 16 kHz mono PCM and sends it as raw little-endian i16 bytes.
use serde::Serialize;
use sherpa_rs::whisper::{WhisperConfig, WhisperRecognizer};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::{ipc::Request, AppHandle, Emitter, Manager};

const BASE: &str = "https://huggingface.co/csukuangfj/sherpa-onnx-whisper-base/resolve/main/";
const FILES: [&str; 3] = ["base-encoder.int8.onnx", "base-decoder.int8.onnx", "base-tokens.txt"];
/// Whisper reads at most 30 s at once.
const CHUNK: usize = 16_000 * 28;

#[derive(Default)]
pub struct Voice {
    rec: Mutex<Option<WhisperRecognizer>>,
    lang: Mutex<String>,
}

#[derive(Serialize, Clone)]
struct Progress {
    done: u64,
    total: u64,
}

fn model_dir(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app.path().app_local_data_dir().map_err(|e| e.to_string())?.join("voice").join("whisper-base"))
}

fn installed(dir: &std::path::Path) -> bool {
    FILES.iter().all(|f| dir.join(f).is_file())
}

#[tauri::command]
pub fn voice_status(app: AppHandle) -> Result<bool, String> {
    Ok(installed(&model_dir(&app)?))
}

#[tauri::command]
pub async fn voice_download(app: AppHandle) -> Result<(), String> {
    let dir = model_dir(&app)?;
    tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let mut sizes = Vec::new();
        for f in FILES {
            let r = ureq::head(&format!("{BASE}{f}")).call().map_err(|e| format!("{f}: {e}"))?;
            sizes.push(r.header("content-length").and_then(|v| v.parse::<u64>().ok()).unwrap_or(0));
        }
        let total: u64 = sizes.iter().sum();
        let mut done = 0u64;
        let mut last = 0u64;
        for f in FILES {
            let target = dir.join(f);
            if target.is_file() {
                done += std::fs::metadata(&target).map(|m| m.len()).unwrap_or(0);
                continue;
            }
            let part = dir.join(format!("{f}.part"));
            let resp = ureq::get(&format!("{BASE}{f}")).call().map_err(|e| format!("{f}: {e}"))?;
            let mut reader = resp.into_reader();
            let mut out = std::fs::File::create(&part).map_err(|e| e.to_string())?;
            let mut buf = vec![0u8; 256 * 1024];
            loop {
                let n = reader.read(&mut buf).map_err(|e| e.to_string())?;
                if n == 0 {
                    break;
                }
                out.write_all(&buf[..n]).map_err(|e| e.to_string())?;
                done += n as u64;
                if done - last > 512 * 1024 {
                    last = done;
                    let _ = app.emit("voice-progress", Progress { done, total });
                }
            }
            drop(out);
            std::fs::rename(&part, &target).map_err(|e| e.to_string())?;
        }
        let _ = app.emit("voice-progress", Progress { done: total, total });
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Body: raw i16 LE mono 16 kHz PCM. Header `x-lang`: ISO code such as `pt`, or `auto`.
#[tauri::command]
pub async fn voice_transcribe(app: AppHandle, request: Request<'_>) -> Result<String, String> {
    let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else {
        return Err("expected raw audio bytes".into());
    };
    let samples: Vec<f32> = bytes.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0).collect();
    let lang = request.headers().get("x-lang").and_then(|v| v.to_str().ok()).unwrap_or("auto").to_string();
    let dir = model_dir(&app)?;
    tauri::async_runtime::spawn_blocking(move || -> Result<String, String> {
        let voice = app.state::<Voice>();
        let mut rec = voice.rec.lock().unwrap();
        let mut cur = voice.lang.lock().unwrap();
        if rec.is_none() || *cur != lang {
            if !installed(&dir) {
                return Err("modelo de voz não instalado".into());
            }
            let p = |f: &str| dir.join(f).to_string_lossy().into_owned();
            let cfg = WhisperConfig {
                encoder: p(FILES[0]),
                decoder: p(FILES[1]),
                tokens: p(FILES[2]),
                language: if lang == "auto" { String::new() } else { lang.clone() },
                num_threads: Some(4),
                ..Default::default()
            };
            *rec = Some(WhisperRecognizer::new(cfg).map_err(|e| e.to_string())?);
            *cur = lang;
        }
        let r = rec.as_mut().unwrap();
        let text: Vec<String> = samples.chunks(CHUNK).filter(|c| c.len() > 1600).map(|c| r.transcribe(16_000, c).text.trim().to_string()).collect();
        Ok(text.join(" ").trim().to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}
