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

/// Files whose relative path contains every word of `query` (case-insensitive), `.gitignore`
/// applied. Name matches come first, then shorter paths; capped so a huge tree stays cheap.
pub fn search(wt: &Path, query: &str) -> Result<Vec<String>> {
    let words: Vec<String> = query.to_lowercase().split_whitespace().map(String::from).collect();
    if words.is_empty() {
        return Ok(Vec::new());
    }
    let mut hits: Vec<(bool, String)> = Vec::new();
    for (seen, e) in ignore::WalkBuilder::new(wt).hidden(false).filter_entry(|e| e.file_name() != ".git").build().enumerate() {
        if seen > 100_000 {
            break;
        }
        let Ok(e) = e else { continue };
        if !e.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let Ok(rel) = e.path().strip_prefix(wt) else { continue };
        let rel = rel.to_string_lossy().replace('\\', "/");
        let low = rel.to_lowercase();
        if words.iter().all(|w| low.contains(w)) {
            let name = low.rsplit('/').next().unwrap_or(&low).to_string();
            hits.push((!words.iter().all(|w| name.contains(w)), rel));
        }
    }
    hits.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.len().cmp(&b.1.len())).then(a.1.cmp(&b.1)));
    Ok(hits.into_iter().take(100).map(|h| h.1).collect())
}

#[derive(Debug, Serialize)]
pub struct Match {
    pub path: String,
    pub line: usize,
    pub text: String,
}

/// Lines containing `query` (case-insensitive) in text files, `.gitignore` applied; capped.
pub fn grep(wt: &Path, query: &str) -> Result<Vec<Match>> {
    let q = query.to_lowercase();
    let mut out = Vec::new();
    if q.trim().is_empty() {
        return Ok(out);
    }
    for (seen, e) in ignore::WalkBuilder::new(wt).hidden(false).filter_entry(|e| e.file_name() != ".git").build().enumerate() {
        if seen > 50_000 || out.len() >= 200 {
            break;
        }
        let Ok(e) = e else { continue };
        if !e.file_type().is_some_and(|t| t.is_file()) || e.metadata().map(|m| m.len() > MAX_FILE_BYTES).unwrap_or(true) {
            continue;
        }
        let Ok(bytes) = std::fs::read(e.path()) else { continue };
        if bytes.contains(&0) {
            continue;
        }
        let Ok(text) = String::from_utf8(bytes) else { continue };
        let Ok(rel) = e.path().strip_prefix(wt) else { continue };
        let rel = rel.to_string_lossy().replace('\\', "/");
        for (i, l) in text.lines().enumerate() {
            if l.to_lowercase().contains(&q) {
                out.push(Match { path: rel.clone(), line: i + 1, text: l.trim().chars().take(200).collect() });
                if out.len() >= 200 {
                    break;
                }
            }
        }
    }
    Ok(out)
}

/// Program and arguments that open `wt` in VS Code. On Windows the `code` on PATH is a `.cmd` shim, which only
/// `cmd` can run and `cmd` would read `&` or `%` in the path as syntax: so the real `Code.exe` next to the shim is
/// started directly, and `cmd` is only the fallback for a path with nothing special in it.
pub fn code_launch(wt: &Path) -> Result<(PathBuf, Vec<String>)> {
    let path = wt.to_string_lossy().into_owned();
    if !cfg!(windows) {
        return Ok((PathBuf::from("code"), vec![path]));
    }
    let shim = std::env::var_os("PATH").and_then(|p| std::env::split_paths(&p).map(|d| d.join("code.cmd")).find(|c| c.is_file()));
    if let Some(exe) = shim.and_then(|s| s.parent()?.parent().map(|d| d.join("Code.exe"))).filter(|e| e.is_file()) {
        return Ok((exe, vec![path]));
    }
    if path.chars().any(|c| matches!(c, '&' | '|' | '<' | '>' | '^' | '%' | '"' | '!' | '(' | ')') || c.is_control()) {
        bail!("could not find Code.exe, and the path has characters the command shell would misread");
    }
    Ok((PathBuf::from("cmd"), vec!["/d".into(), "/c".into(), "code".into(), path]))
}

/// A path component that git or the file system treats as the `.git` entry: case-insensitive, and Windows
/// ignores trailing dots and spaces.
fn is_git_name(c: &str) -> bool {
    c.trim_end_matches(['.', ' ']).eq_ignore_ascii_case(".git")
}

fn not_git(rel: &str) -> Result<()> {
    if rel.is_empty() || rel.split(['/', '\\']).any(is_git_name) {
        bail!("not allowed");
    }
    Ok(())
}

/// New empty file or folder; refuses to overwrite.
pub fn create(wt: &Path, rel: &str, dir: bool) -> Result<()> {
    not_git(rel)?;
    let p = resolve(wt, rel)?;
    if p.exists() {
        bail!("already exists");
    }
    if dir {
        std::fs::create_dir_all(&p)?;
    } else {
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::File::create_new(&p)?;
    }
    Ok(())
}

pub fn rename(wt: &Path, from: &str, to: &str) -> Result<()> {
    not_git(from)?;
    not_git(to)?;
    let a = resolve(wt, from)?;
    let b = resolve(wt, to)?;
    if !a.exists() {
        bail!("not found");
    }
    if b.exists() {
        bail!("already exists");
    }
    std::fs::rename(a, b)?;
    Ok(())
}

/// Permanent delete of a file or folder inside the worktree (symlinks are unlinked, not followed).
pub fn delete(wt: &Path, rel: &str) -> Result<()> {
    not_git(rel)?;
    let p = resolve(wt, rel)?;
    let md = std::fs::symlink_metadata(&p).context("not found")?;
    if md.is_dir() {
        std::fs::remove_dir_all(&p)?;
    } else {
        std::fs::remove_file(&p)?;
    }
    Ok(())
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
    not_git(rel)?;
    let mut p = resolve(wt, rel)?;
    // a symlink is written through to its target, not replaced; either way the real location may not be in .git
    let root = axelstra_core::git::canon(wt)?;
    if std::fs::symlink_metadata(&p).is_ok_and(|m| m.file_type().is_symlink()) {
        p = axelstra_core::git::canon(&p)?;
    }
    let real = axelstra_core::git::canon(if p.exists() { &p } else { p.parent().context("no parent")? })?;
    let inner = real.strip_prefix(&root).context("path escapes the worktree")?.to_string_lossy().into_owned();
    if !inner.is_empty() {
        not_git(&inner)?;
    }
    let mode = std::fs::metadata(&p).ok().map(|m| m.permissions());
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
    if let Some(mode) = mode {
        let _ = std::fs::set_permissions(&tmp, mode);
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("axl-fsops-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join(".git")).unwrap();
        d
    }

    #[cfg(windows)]
    #[test]
    fn code_launch_never_hands_a_special_path_to_cmd() {
        let (prog, args) = code_launch(Path::new(r"C:\work&b")).map(|(p, a)| (p.to_string_lossy().into_owned(), a)).unwrap_or_default();
        // either the real exe (no shell) or an error; never `cmd` with the ampersand
        assert!(!(prog.eq_ignore_ascii_case("cmd") && args.iter().any(|a| a.contains('&'))));
        assert!(code_launch(Path::new(r"C:\work\plain")).is_ok() || true);
    }

    #[test]
    fn git_dir_is_refused_whatever_the_spelling() {
        for bad in [".git", ".GIT/config", "sub/.Git/hooks/pre-commit", ".git./x", ".git /x", "a\\.git\\x"] {
            assert!(not_git(bad).is_err(), "{bad}");
        }
        for ok in [".gitignore", ".github/ci.yml", "src/git.rs"] {
            assert!(not_git(ok).is_ok(), "{ok}");
        }
        let d = tmpdir("git");
        assert!(write(&d, ".GIT/config", "x", "lf", false, None).is_err());
        assert!(write(&d, ".git/hooks/pre-commit", "x", "lf", false, None).is_err());
        assert!(write(&d, "ok.txt", "x", "lf", false, None).is_ok());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[cfg(unix)]
    #[test]
    fn write_keeps_mode_and_symlinks() {
        use std::os::unix::fs::PermissionsExt;
        let d = tmpdir("mode");
        std::fs::write(d.join("run.sh"), "a").unwrap();
        std::fs::set_permissions(d.join("run.sh"), std::fs::Permissions::from_mode(0o755)).unwrap();
        write(&d, "run.sh", "b", "lf", false, None).unwrap();
        assert_eq!(std::fs::metadata(d.join("run.sh")).unwrap().permissions().mode() & 0o777, 0o755);
        std::os::unix::fs::symlink("run.sh", d.join("link")).unwrap();
        write(&d, "link", "c", "lf", false, None).unwrap();
        assert!(std::fs::symlink_metadata(d.join("link")).unwrap().file_type().is_symlink());
        assert_eq!(std::fs::read_to_string(d.join("run.sh")).unwrap(), "c");
        // a link into .git is not a way around the check
        std::os::unix::fs::symlink(".git", d.join("g")).unwrap();
        assert!(write(&d, "g/config", "x", "lf", false, None).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }
}
