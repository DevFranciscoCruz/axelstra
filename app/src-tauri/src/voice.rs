//! Local speech to text: Whisper (base, int8) through sherpa-onnx. Audio never leaves the machine.
//! The webview records 16 kHz mono PCM and sends it as raw little-endian i16 bytes.
use serde::Serialize;
use sha2::Digest;
use sherpa_rs::whisper::{WhisperConfig, WhisperRecognizer};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::{ipc::Request, AppHandle, Emitter, Manager};

/// Pinned to one revision of the model repo, and every file to its SHA-256: a changed or swapped download is refused.
const BASE: &str = "https://huggingface.co/csukuangfj/sherpa-onnx-whisper-base/resolve/bb53ee204431c90d314c1cc08d28d23e5b7927cc/";
const FILES: [(&str, &str); 3] = [
    ("base-encoder.int8.onnx", "0b8fb1304b6109976038efff5ace81720e00386f3ff6b54ee8c75291ca0a1e11"),
    ("base-decoder.int8.onnx", "9759d217388a01b3a4c7c15533201067b48ae819c4daafc8624e64b9409dc02d"),
    ("base-tokens.txt", "b34b360dbb493e781e479794586d661700670d65564001f23024971d1f2fa126"),
];
/// Written once every file matched its hash; a model without it (older install) is checked again.
const VERIFIED: &str = ".verified";
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
    dir.join(VERIFIED).is_file() && FILES.iter().all(|(f, _)| dir.join(f).is_file())
}

fn sha256_of(r: &mut impl Read) -> std::io::Result<String> {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let n = r.read(&mut buf)?;
        if n == 0 {
            return Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect());
        }
        h.update(&buf[..n]);
    }
}

fn file_matches(path: &std::path::Path, want: &str) -> bool {
    std::fs::File::open(path).and_then(|mut f| sha256_of(&mut f)).is_ok_and(|h| h == want)
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
        let _ = std::fs::remove_file(dir.join(VERIFIED));
        for (f, _) in FILES {
            let r = ureq::head(&format!("{BASE}{f}")).call().map_err(|e| format!("{f}: {e}"))?;
            sizes.push(r.header("content-length").and_then(|v| v.parse::<u64>().ok()).unwrap_or(0));
        }
        let total: u64 = sizes.iter().sum();
        let mut done = 0u64;
        let mut last = 0u64;
        for (f, want) in FILES {
            let target = dir.join(f);
            if target.is_file() {
                if file_matches(&target, want) {
                    done += std::fs::metadata(&target).map(|m| m.len()).unwrap_or(0);
                    continue;
                }
                let _ = std::fs::remove_file(&target);
            }
            let part = dir.join(format!("{f}.part"));
            let resp = ureq::get(&format!("{BASE}{f}")).call().map_err(|e| format!("{f}: {e}"))?;
            let mut reader = resp.into_reader();
            let mut out = std::fs::File::create(&part).map_err(|e| e.to_string())?;
            let mut buf = vec![0u8; 256 * 1024];
            let mut hasher = sha2::Sha256::new();
            loop {
                let n = reader.read(&mut buf).map_err(|e| e.to_string())?;
                if n == 0 {
                    break;
                }
                hasher.update(&buf[..n]);
                out.write_all(&buf[..n]).map_err(|e| e.to_string())?;
                done += n as u64;
                if done - last > 512 * 1024 {
                    last = done;
                    let _ = app.emit("voice-progress", Progress { done, total });
                }
            }
            drop(out);
            let got: String = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect();
            if got != want {
                let _ = std::fs::remove_file(&part);
                return Err(format!("{f}: o ficheiro descarregado não corresponde ao hash esperado"));
            }
            std::fs::rename(&part, &target).map_err(|e| e.to_string())?;
        }
        std::fs::write(dir.join(VERIFIED), "").map_err(|e| e.to_string())?;
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
    let samples: Vec<f32> = bytes.as_chunks::<2>().0.iter().map(|b| i16::from_le_bytes(*b) as f32 / 32768.0).collect();
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
                encoder: p(FILES[0].0),
                decoder: p(FILES[1].0),
                tokens: p(FILES[2].0),
                language: if lang == "auto" { String::new() } else { lang.clone() },
                num_threads: Some(4),
                ..Default::default()
            };
            *rec = Some(WhisperRecognizer::new(cfg).map_err(|e| e.to_string())?);
            *cur = lang;
        }
        let r = rec.as_mut().unwrap();
        let text: Vec<String> =
            samples.chunks(CHUNK).filter(|c| c.len() > 1600).map(|c| r.transcribe(16_000, c).text.trim().to_string()).collect();
        Ok(text.join(" ").trim().to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_matches_the_known_digest() {
        // sha256("abc")
        assert_eq!(sha256_of(&mut &b"abc"[..]).unwrap(), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        let f = std::env::temp_dir().join(format!("axl-voice-{}", std::process::id()));
        std::fs::write(&f, b"abc").unwrap();
        assert!(file_matches(&f, "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"));
        assert!(!file_matches(&f, FILES[2].1));
        let _ = std::fs::remove_file(&f);
    }

    #[test]
    fn every_pinned_hash_is_a_sha256_hex_digest() {
        for (_, h) in FILES {
            assert!(h.len() == 64 && h.chars().all(|c| c.is_ascii_hexdigit()));
        }
    }
}
