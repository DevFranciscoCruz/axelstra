//! File access for the editor. Everything is confined to a registered worktree:
//! relative paths only, symlinks must not leave it, and writes are atomic and
//! refuse to overwrite a file that changed on disk since it was read.

use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::io::Write;
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

/// Names Windows resolves to something else: `.git::$INDEX_ALLOCATION` (stream syntax) and the 8.3 short
/// form `GIT~1`.
fn win_name_bad(c: &str) -> bool {
    c.contains(':') || c.as_bytes().windows(2).any(|w| w[0] == b'~' && w[1].is_ascii_digit())
}

/// Normalized relative path for a mutator: `.` segments dropped, at least one `Normal` component (so `.`
/// never names the worktree itself), `.git` and, on Windows, stream/short-name spellings refused.
fn norm_rel(rel: &str) -> Result<String> {
    rel_ok(rel)?;
    let parts: Vec<String> = Path::new(rel)
        .components()
        .filter_map(|c| match c {
            Component::Normal(n) => Some(n.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();
    if parts.is_empty() {
        bail!("invalid path");
    }
    if cfg!(windows) && parts.iter().any(|c| win_name_bad(c)) {
        bail!("not allowed");
    }
    let n = parts.join("/");
    not_git(&n)?;
    Ok(n)
}

fn is_link(md: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if md.file_attributes() & 0x400 != 0 {
            return true; // FILE_ATTRIBUTE_REPARSE_POINT: symlink or junction
        }
    }
    md.file_type().is_symlink()
}

/// Removes the link itself, never what it points to. On Windows a link to a folder (symlink or junction) is a
/// directory entry and goes with `remove_dir`; `remove_file` fails on it.
fn unlink(p: &Path, md: &std::fs::Metadata) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
        if md.file_attributes() & FILE_ATTRIBUTE_DIRECTORY != 0 {
            return std::fs::remove_dir(p);
        }
    }
    let _ = md;
    std::fs::remove_file(p)
}

/// Resolves a mutation target: normalized, no symlink/reparse directory on the way, and the real parent
/// directory is not inside `.git`. Only the parent is resolved: the last component itself may be a link, even one
/// that points outside the worktree (it is replaced or unlinked, never followed), so it can still be deleted or renamed.
fn mutable(wt: &Path, rel: &str) -> Result<PathBuf> {
    let n = norm_rel(rel)?;
    let root = axelstra_core::git::canon(wt)?;
    let full = root.join(&n);
    let parts: Vec<&str> = n.split('/').collect();
    let mut cur = root.clone();
    for part in &parts[..parts.len() - 1] {
        cur.push(part);
        match std::fs::symlink_metadata(&cur) {
            Ok(md) if is_link(&md) => bail!("path goes through a link"),
            Ok(_) => {}
            Err(_) => break,
        }
    }
    let parent = full.parent().context("no parent")?;
    // a parent that does not exist yet (`create` makes it) is checked from its nearest existing ancestor
    let probe = parent.ancestors().find(|a| a.exists()).unwrap_or(&root);
    let real = axelstra_core::git::canon(probe)?;
    let inner = real.strip_prefix(&root).context("path escapes the worktree")?.to_string_lossy().into_owned();
    if !inner.is_empty() {
        not_git(&inner)?;
    }
    Ok(full)
}

/// New empty file or folder; refuses to overwrite.
pub fn create(wt: &Path, rel: &str, dir: bool) -> Result<()> {
    let p = mutable(wt, rel)?;
    // a dangling link is "there" too: creating through it would write wherever it points
    if std::fs::symlink_metadata(&p).is_ok() {
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
    let a = mutable(wt, from)?;
    let b = mutable(wt, to)?;
    if std::fs::symlink_metadata(&a).is_err() {
        bail!("not found");
    }
    if std::fs::symlink_metadata(&b).is_ok() {
        bail!("already exists");
    }
    std::fs::rename(a, b)?;
    Ok(())
}

/// Permanent delete of a file or folder inside the worktree (symlinks are unlinked, not followed).
pub fn delete(wt: &Path, rel: &str) -> Result<()> {
    let p = mutable(wt, rel)?;
    let md = std::fs::symlink_metadata(&p).context("not found")?;
    if is_link(&md) {
        unlink(&p, &md)?;
    } else if md.is_dir() {
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

/// Fresh temp file next to `p`, created with `create_new` under a random name so a planted link is never followed.
fn tmp_file(p: &Path) -> Result<(PathBuf, std::fs::File)> {
    let dir = p.parent().context("no parent")?;
    for _ in 0..8 {
        let mut r = [0u8; 8];
        getrandom::getrandom(&mut r).map_err(|e| anyhow::anyhow!("no randomness: {e}"))?;
        let name: String = r.iter().map(|b| format!("{b:02x}")).collect();
        let t = dir.join(format!(".{name}.axl-tmp"));
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&t) {
            Ok(f) => return Ok((t, f)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.into()),
        }
    }
    bail!("could not create a temporary file")
}

pub fn write(wt: &Path, rel: &str, text: &str, eol: &str, bom: bool, base_hash: Option<&str>) -> Result<Saved> {
    let mut p = mutable(wt, rel)?;
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
    let (tmp, mut f) = tmp_file(&p)?;
    if let Err(e) = f.write_all(&bytes).and_then(|()| f.sync_all()) {
        drop(f);
        let _ = std::fs::remove_file(&tmp);
        return Err(e.into());
    }
    drop(f);
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
        // a plain path is passed through untouched, whichever launcher is used
        let (prog, args) = code_launch(Path::new(r"C:\work\plain")).expect("plain path must launch");
        assert!(args.iter().any(|a| a.contains("plain")), "{prog:?} {args:?}");
    }

    #[test]
    fn dot_never_names_the_worktree() {
        let d = tmpdir("dot");
        std::fs::write(d.join("keep.txt"), "a").unwrap();
        for bad in [".", "./", "./.", "", "a/..", ".."] {
            assert!(delete(&d, bad).is_err(), "delete {bad:?}");
            assert!(create(&d, bad, true).is_err(), "create {bad:?}");
            assert!(write(&d, bad, "x", "lf", false, None).is_err(), "write {bad:?}");
            assert!(rename(&d, bad, "other").is_err(), "rename from {bad:?}");
            assert!(rename(&d, "keep.txt", bad).is_err(), "rename to {bad:?}");
        }
        assert!(d.join(".git").is_dir() && d.join("keep.txt").is_file());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn delete_and_rename_work_and_guard_git() {
        let d = tmpdir("delren");
        std::fs::create_dir_all(d.join("sub/in")).unwrap();
        std::fs::write(d.join("sub/in/f.txt"), "a").unwrap();
        std::fs::write(d.join("a.txt"), "a").unwrap();
        rename(&d, "./a.txt", "b.txt").unwrap();
        assert!(!d.join("a.txt").exists() && d.join("b.txt").is_file());
        assert!(rename(&d, "b.txt", "sub/in/f.txt").is_err(), "no overwrite");
        assert!(rename(&d, "missing", "c.txt").is_err());
        assert!(rename(&d, "b.txt", ".git/hooks").is_err());
        assert!(rename(&d, ".git", "x").is_err());
        assert!(delete(&d, ".git").is_err() && delete(&d, ".GIT/config").is_err());
        delete(&d, "b.txt").unwrap();
        delete(&d, "sub").unwrap();
        assert!(!d.join("b.txt").exists() && !d.join("sub").exists());
        assert!(delete(&d, "sub").is_err());
        assert!(delete(&d, "../x").is_err() && delete(&d, "/etc").is_err());
        assert!(d.join(".git").is_dir());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn windows_stream_and_short_names_are_flagged() {
        for bad in [".git::$INDEX_ALLOCATION", "GIT~1", "a:b", "x~12"] {
            assert!(win_name_bad(bad), "{bad}");
        }
        for ok in ["git", "a~b", "plain.txt", "~"] {
            assert!(!win_name_bad(ok), "{ok}");
        }
        if cfg!(windows) {
            let d = tmpdir("winnames");
            assert!(create(&d, "GIT~1/hooks", true).is_err());
            assert!(create(&d, ".git::$INDEX_ALLOCATION/x", false).is_err());
            let _ = std::fs::remove_dir_all(&d);
        }
    }

    #[test]
    fn write_uses_a_random_temp_and_leaves_none_behind() {
        let d = tmpdir("tmp");
        std::fs::write(d.join("f.txt"), "a").unwrap();
        // the old predictable temp name must not be touched
        std::fs::write(d.join("f.txt.axl-tmp"), "planted").unwrap();
        write(&d, "f.txt", "b", "lf", false, None).unwrap();
        assert_eq!(std::fs::read_to_string(d.join("f.txt")).unwrap(), "b");
        assert_eq!(std::fs::read_to_string(d.join("f.txt.axl-tmp")).unwrap(), "planted");
        let left = std::fs::read_dir(&d).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().starts_with('.') && e.file_name().to_string_lossy().ends_with(".axl-tmp")).count();
        assert_eq!(left, 0);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Makes a directory link; None when the host does not allow it (Windows without privilege).
    fn dir_link(target: &Path, link: &Path) -> Option<()> {
        #[cfg(unix)]
        let r = std::os::unix::fs::symlink(target, link);
        #[cfg(windows)]
        let r = std::os::windows::fs::symlink_dir(target, link);
        r.ok()
    }

    #[test]
    fn mutators_refuse_to_go_through_a_link_to_git() {
        let d = tmpdir("link");
        if dir_link(&d.join(".git"), &d.join("g")).is_none() {
            eprintln!("skipped: cannot create symlinks here");
            let _ = std::fs::remove_dir_all(&d);
            return;
        }
        std::fs::write(d.join("evil.sh"), "x").unwrap();
        assert!(rename(&d, "evil.sh", "g/pre-commit").is_err());
        assert!(create(&d, "g/hooks", true).is_err());
        assert!(create(&d, "g/f", false).is_err());
        assert!(write(&d, "g/config", "x", "lf", false, None).is_err());
        assert!(delete(&d, "g/config").is_err());
        assert!(!d.join(".git/pre-commit").exists() && d.join("evil.sh").exists());
        // the link itself can still be unlinked, and .git survives
        delete(&d, "g").unwrap();
        assert!(d.join(".git").is_dir());
        let _ = std::fs::remove_dir_all(&d);
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

    /// A link to a folder that lives outside `wt`; a junction on Windows (no privilege needed), else a symlink.
    fn outside_link(wt: &Path, name: &str) -> Option<PathBuf> {
        let out = std::env::temp_dir().join(format!("axl-fsops-out-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&out);
        std::fs::create_dir_all(&out).unwrap();
        std::fs::write(out.join("precious.txt"), "keep").unwrap();
        let link = wt.join("lnk");
        #[cfg(windows)]
        let made = std::process::Command::new("cmd").args(["/C", "mklink", "/J"]).arg(&link).arg(&out).output().is_ok_and(|o| o.status.success());
        #[cfg(not(windows))]
        let made = dir_link(&out, &link).is_some();
        made.then_some(out)
    }

    #[test]
    fn a_link_pointing_outside_can_be_deleted_and_renamed_without_touching_its_target() {
        let d = tmpdir("outlink");
        let Some(out) = outside_link(&d, "del") else {
            eprintln!("skipped: cannot create links here");
            let _ = std::fs::remove_dir_all(&d);
            return;
        };
        rename(&d, "lnk", "moved").unwrap();
        assert!(std::fs::symlink_metadata(d.join("moved")).is_ok() && std::fs::symlink_metadata(d.join("lnk")).is_err());
        // reading or writing through it stays refused
        assert!(read(&d, "moved/precious.txt").is_err());
        assert!(write(&d, "moved/precious.txt", "x", "lf", false, None).is_err());
        assert!(create(&d, "moved/new.txt", false).is_err());
        delete(&d, "moved").unwrap();
        assert!(std::fs::symlink_metadata(d.join("moved")).is_err());
        assert_eq!(std::fs::read_to_string(out.join("precious.txt")).unwrap(), "keep");
        let _ = std::fs::remove_dir_all(&d);
        let _ = std::fs::remove_dir_all(&out);
    }

    #[test]
    fn create_refuses_a_dangling_link_instead_of_writing_through_it() {
        let d = tmpdir("dangling");
        let ghost = std::env::temp_dir().join(format!("axl-fsops-ghost-{}", std::process::id()));
        let _ = std::fs::remove_file(&ghost);
        #[cfg(unix)]
        let made = std::os::unix::fs::symlink(&ghost, d.join("l")).is_ok();
        #[cfg(windows)]
        let made = std::os::windows::fs::symlink_file(&ghost, d.join("l")).is_ok();
        if made {
            assert!(create(&d, "l", false).is_err());
            assert!(!ghost.exists());
        }
        let _ = std::fs::remove_dir_all(&d);
    }
}
