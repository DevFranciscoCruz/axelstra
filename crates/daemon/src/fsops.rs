//! File access for the editor. Everything is confined to a registered worktree:
//! relative paths only, symlinks must not leave it, and writes are atomic and
//! refuse to overwrite a file that changed on disk since it was read.

use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

pub const MAX_FILE_BYTES: u64 = 2 << 20;

#[derive(Debug, Serialize)]
pub struct Entry {
    pub name: String,
    pub dir: bool,
}

#[derive(Debug, Serialize)]
pub struct Opened {
    /// always LF inside the editor; `eol` says what to write back
    pub text: String,
    pub eol: &'static str,
    pub bom: bool,
    pub hash: String,
}

pub enum Saved {
    Ok { hash: String },
    Conflict { disk_hash: String },
}

/// FNV-1a over the raw bytes: cheap change detection, not security.
pub fn hash(bytes: &[u8]) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

fn rel_ok(rel: &str) -> Result<()> {
    let p = Path::new(rel);
    if p.is_absolute() || p.components().any(|c| !matches!(c, Component::Normal(_) | Component::CurDir)) {
        bail!("invalid path");
    }
    Ok(())
}

/// `wt/rel`, verified to stay inside `wt` even through symlinks.
/// The target may not exist yet (new file): then its parent is checked.
fn resolve(wt: &Path, rel: &str) -> Result<PathBuf> {
    rel_ok(rel)?;
    let root = axelstra_core::git::canon(wt)?;
    let full = root.join(rel);
    let probe = if full.exists() { full.clone() } else { full.parent().context("no parent")?.to_path_buf() };
    let real = axelstra_core::git::canon(&probe)?;
    if !real.starts_with(&root) {
        bail!("path escapes the worktree");
    }
    Ok(full)
}

/// One directory level, `.gitignore` applied, `.git` hidden, folders first.
pub fn list(wt: &Path, rel: &str) -> Result<Vec<Entry>> {
    let dir = resolve(wt, rel)?;
    if !dir.is_dir() {
        bail!("not a directory");
    }
    let mut out = Vec::new();
    for e in ignore::WalkBuilder::new(&dir).max_depth(Some(1)).hidden(false).build() {
        let e = e?;
        if e.depth() == 0 {
            continue;
        }
        let name = e.file_name().to_string_lossy().to_string();
        if name == ".git" {
            continue;
        }
        out.push(Entry { name, dir: e.file_type().is_some_and(|t| t.is_dir()) });
    }
    out.sort_by(|a, b| b.dir.cmp(&a.dir).then(a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    Ok(out)
}

pub fn read(wt: &Path, rel: &str) -> Result<Opened> {
    let p = resolve(wt, rel)?;
    let md = std::fs::metadata(&p).context("file not found")?;
    if !md.is_file() {
        bail!("not a file");
    }
    if md.len() > MAX_FILE_BYTES {
        bail!("file is larger than {} MB", MAX_FILE_BYTES >> 20);
    }
    let bytes = std::fs::read(&p)?;
    if bytes.contains(&0) {
        bail!("binary file");
    }
    let h = hash(&bytes);
    let text = String::from_utf8(bytes).map_err(|_| anyhow::anyhow!("not valid UTF-8 text"))?;
    let (text, bom) = match text.strip_prefix('\u{feff}') {
        Some(t) => (t.to_string(), true),
        None => (text, false),
    };
    let crlf = text.contains("\r\n");
    Ok(Opened { text: if crlf { text.replace("\r\n", "\n") } else { text }, eol: if crlf { "crlf" } else { "lf" }, bom, hash: h })
}

pub fn stat_hash(wt: &Path, rel: &str) -> Result<Option<String>> {
    let p = resolve(wt, rel)?;
    match std::fs::read(&p) {
        Ok(b) => Ok(Some(hash(&b))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

pub fn write(wt: &Path, rel: &str, text: &str, eol: &str, bom: bool, base_hash: Option<&str>) -> Result<Saved> {
    let p = resolve(wt, rel)?;
    if text.len() as u64 > MAX_FILE_BYTES {
        bail!("file is larger than {} MB", MAX_FILE_BYTES >> 20);
    }
    // someone (often an agent) changed it since we read it: do not clobber
    if let (Some(base), Ok(disk)) = (base_hash, std::fs::read(&p)) {
        let d = hash(&disk);
        if d != base {
            return Ok(Saved::Conflict { disk_hash: d });
        }
    }
    let mut data = String::new();
    if bom {
        data.push('\u{feff}');
    }
    if eol == "crlf" {
        data.push_str(&text.replace('\n', "\r\n"));
    } else {
        data.push_str(text);
    }
    let bytes = data.into_bytes();
    let tmp = p.with_extension(format!("{}.axl-tmp", p.extension().map(|e| e.to_string_lossy()).unwrap_or_default()));
    std::fs::write(&tmp, &bytes)?;
    // another process may hold the target open for a moment (editors, scanners)
    let mut last = None;
    for _ in 0..5 {
        match std::fs::rename(&tmp, &p) {
            Ok(()) => return Ok(Saved::Ok { hash: hash(&bytes) }),
            Err(e) => {
                last = Some(e);
                std::thread::sleep(Duration::from_millis(60));
            }
        }
    }
    let _ = std::fs::remove_file(&tmp);
    Err(last.unwrap().into())
}
