//! The file explorer's local file access, served by this host without the daemon (see `docs/EXPLORER.md`).
//!
//! A slow or busy daemon must not stall a folder that sits on this disk. Explicit commands (no generic file API, no fs
//! plugin scope) list a folder, stat and read a text file, open or reveal it in the OS, and watch the visible worktree.
//! Every path is confined here, from disk state alone: the allowed roots are the registered projects (`projects.json`
//! in the data dir) and their linked worktrees (`<project>/.git/worktrees/*/gitdir`), both plain file reads. The
//! webview names a root and a path relative to it; a path outside the roots, one that leaves them through `..` or a
//! symlink, a Windows reserved name, a UNC or device path, a remote (`@host:`) path: all refused with a coded error.
//! `not_local` means "ask the daemon instead" (remote hosts, UNC projects); the other codes are final answers.

use crate::CallError;
use anyhow::Result;
use axelstra_core::coded;
use axelstra_daemon::fsops::{self, Entry, Opened};
use axelstra_daemon::pathopen::{self, Action, PathInfo};
use serde::Serialize;
use serde_json::Value;
use std::collections::{HashMap, VecDeque};
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use tauri::ipc::Channel;

/// Entries read from one folder before the listing stops and says `truncated`.
pub const MAX_ENTRIES: usize = 20_000;
/// Entries per streamed chunk.
const CHUNK: usize = 256;

// ---------------------------------------------------------------- roots

/// Paths of the projects in `projects.json` (the envelope `{"v":N,"data":[...]}` or a bare list), read-only: a file
/// that does not parse is left alone (the daemon moves it aside and restores a backup), the newest readable backup
/// `.1`..`.3` answers meanwhile.
fn project_paths(store: &Path) -> Vec<String> {
    let name = store.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let candidates = std::iter::once(store.to_path_buf()).chain((1..=3).map(|n| store.with_file_name(format!("{name}.{n}"))));
    for f in candidates {
        let Ok(text) = std::fs::read_to_string(&f) else { continue };
        let Ok(v) = serde_json::from_str::<Value>(&text) else { continue };
        let list = match v {
            Value::Object(mut m) if m.contains_key("data") => m.remove("data").unwrap_or(Value::Null),
            other => other,
        };
        if let Value::Array(items) = list {
            return items.iter().filter_map(|p| p["path"].as_str().filter(|s| !s.is_empty()).map(String::from)).collect();
        }
    }
    vec![]
}

/// Worktrees git linked to the repository at `project`: each `.git/worktrees/<id>/gitdir` holds `<worktree>/.git`.
/// Plain file reads; entries of worktrees that are gone are harmless (they fail later with `not_found`).
fn linked_worktrees(project: &Path) -> Vec<PathBuf> {
    let admin = project.join(".git").join("worktrees");
    let Ok(rd) = std::fs::read_dir(&admin) else { return vec![] };
    let mut out = vec![];
    for e in rd.flatten() {
        let Ok(text) = std::fs::read_to_string(e.path().join("gitdir")) else { continue };
        let line = text.lines().next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let gitfile = if Path::new(line).is_absolute() { PathBuf::from(line) } else { e.path().join(line) };
        if let Some(wt) = gitfile.parent() {
            out.push(wt.to_path_buf());
        }
    }
    out
}

/// Every folder the explorer may serve: the projects and their linked worktrees.
pub fn roots(store: &Path) -> Vec<PathBuf> {
    let mut out = vec![];
    for p in project_paths(store) {
        let p = PathBuf::from(p);
        out.extend(linked_worktrees(&p));
        out.push(p);
    }
    out
}

// ---------------------------------------------------------------- confinement

/// UNC (`\\server\share`), device (`\\.\`) and verbatim non-drive (`\\?\UNC\`, `\\?\GLOBALROOT`) paths. The verbatim
/// drive form `\\?\C:\x` is a plain local path.
fn is_unc_or_device(p: &str) -> bool {
    let b = p.as_bytes();
    let slash = |c: u8| c == b'\\' || c == b'/';
    if b.len() < 2 || !slash(b[0]) || !slash(b[1]) {
        return false;
    }
    let rest = &p[2..];
    let verbatim_drive = (rest.starts_with("?\\") || rest.starts_with("?/")) && {
        let r = rest.as_bytes();
        r.len() >= 4 && r[2].is_ascii_alphabetic() && r[3] == b':'
    };
    !verbatim_drive
}

/// Windows device names that open a device instead of a file, with or without an extension (`NUL.txt`), trailing
/// dots and spaces ignored.
pub fn is_reserved(name: &str) -> bool {
    let base = name.split('.').next().unwrap_or("").trim_end_matches(' ').to_uppercase();
    if matches!(base.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$") {
        return true;
    }
    let mut c = base.chars();
    let head: String = c.by_ref().take(3).collect();
    matches!(head.as_str(), "COM" | "LPT") && matches!((c.next(), c.next()), (Some('1'..='9' | '¹' | '²' | '³'), None))
}

fn invalid(path: &str) -> anyhow::Error {
    coded!("localfs.invalid_path", "invalid path: {path}", path = path)
}

/// A path relative to the root: normal components only, no device names, and on Windows no stream (`a:b`) or
/// trailing dot/space spelling that the OS would read as another file.
fn check_rel(rel: &str) -> Result<()> {
    if rel.contains('\0') {
        return Err(invalid(rel));
    }
    let p = Path::new(rel);
    if p.is_absolute() || p.has_root() {
        return Err(invalid(rel));
    }
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::Normal(n) => {
                let n = n.to_string_lossy();
                if cfg!(windows) {
                    if is_reserved(&n) {
                        return Err(coded!("localfs.reserved_name", "reserved name: {path}", path = rel));
                    }
                    if n.contains(':') || n.ends_with('.') || n.ends_with(' ') {
                        return Err(invalid(rel));
                    }
                }
            }
            _ => return Err(invalid(rel)),
        }
    }
    Ok(())
}

fn shown(rel: &str) -> &str {
    if rel.is_empty() {
        "."
    } else {
        rel
    }
}

fn io_err(e: &std::io::Error, rel: &str) -> anyhow::Error {
    match e.kind() {
        ErrorKind::NotFound => coded!("localfs.not_found", "not found: {path}", path = shown(rel)),
        ErrorKind::PermissionDenied => coded!("localfs.permission_denied", "permission denied: {path}", path = shown(rel)),
        _ => coded!("localfs.io_failed", "could not read {path}: {detail}", path = shown(rel), detail = e),
    }
}

/// A path that passed every check: `root` canonical, `full` its canonical child (symlinks resolved).
#[derive(Debug)]
pub struct Located {
    pub root: PathBuf,
    pub full: PathBuf,
}

/// `worktree` (as the page names it) must be one of `roots`; `rel` stays inside it, also through symlinks.
pub fn locate(roots: &[PathBuf], worktree: &str, rel: &str) -> Result<Located> {
    if worktree.is_empty() || worktree.starts_with('@') || (cfg!(windows) && is_unc_or_device(worktree)) {
        return Err(coded!("localfs.not_local", "not a local folder: {path}", path = worktree));
    }
    if worktree.contains('\0') {
        return Err(invalid(worktree));
    }
    let given = roots.iter().find(|r| axelstra_core::paths::same(&r.to_string_lossy(), worktree));
    let Some(given) = given else {
        return Err(coded!("localfs.outside_roots", "not inside a project or worktree: {path}", path = worktree));
    };
    if cfg!(windows) && is_unc_or_device(&given.to_string_lossy()) {
        return Err(coded!("localfs.not_local", "not a local folder: {path}", path = worktree));
    }
    check_rel(rel)?;
    let root = axelstra_core::git::canon(given).map_err(|e| match e.downcast_ref::<std::io::Error>() {
        Some(io) => io_err(io, ""),
        None => coded!("localfs.not_found", "not found: {path}", path = "."),
    })?;
    let full = axelstra_core::git::canon(&root.join(rel)).map_err(|e| match e.downcast_ref::<std::io::Error>() {
        Some(io) => io_err(io, rel),
        None => coded!("localfs.not_found", "not found: {path}", path = shown(rel)),
    })?;
    if !full.starts_with(&root) {
        return Err(coded!("localfs.outside_roots", "not inside a project or worktree: {path}", path = shown(rel)));
    }
    Ok(Located { root, full })
}

// ---------------------------------------------------------------- listing

#[derive(Serialize, Debug)]
pub struct Listing {
    /// folders first, then by name (case-insensitive), `offset`/`limit` applied
    pub entries: Vec<Entry>,
    /// entries read from the folder (at most `MAX_ENTRIES`), before paging
    pub total: usize,
    /// the folder has more entries than were read
    pub truncated: bool,
}

pub struct Limits {
    pub max: usize,
    pub chunk: usize,
}

pub const LIMITS: Limits = Limits { max: MAX_ENTRIES, chunk: CHUNK };

fn cancelled() -> anyhow::Error {
    coded!("localfs.cancelled", "cancelled")
}

/// One folder level, `.gitignore` applied and `.git` hidden (the same view as the daemon's `fs.list`). `on_chunk` gets
/// the raw entries as they are read; the return value is sorted and paged. `cancel` is checked per entry.
pub fn list(
    roots: &[PathBuf],
    worktree: &str,
    rel: &str,
    cancel: &AtomicBool,
    page: (usize, Option<usize>),
    lim: &Limits,
    mut on_chunk: impl FnMut(&[Entry]),
) -> Result<Listing> {
    let at = locate(roots, worktree, rel)?;
    let md = std::fs::metadata(&at.full).map_err(|e| io_err(&e, rel))?;
    if !md.is_dir() {
        return Err(coded!("localfs.not_dir", "not a folder: {path}", path = shown(rel)));
    }
    if cancel.load(Ordering::SeqCst) {
        return Err(cancelled());
    }
    let mut out: Vec<Entry> = Vec::new();
    let mut sent = 0;
    let mut truncated = false;
    for e in ignore::WalkBuilder::new(&at.full).max_depth(Some(1)).hidden(false).build() {
        if cancel.load(Ordering::SeqCst) {
            return Err(cancelled());
        }
        let e = match e {
            Ok(e) => e,
            // the folder itself cannot be read: say so; a single unreadable entry is skipped
            Err(err) if out.is_empty() => match err.io_error() {
                Some(io) if io.kind() == ErrorKind::PermissionDenied => return Err(io_err(io, rel)),
                _ => continue,
            },
            Err(_) => continue,
        };
        if e.depth() == 0 {
            continue;
        }
        let name = e.file_name().to_string_lossy().to_string();
        if name == ".git" {
            continue;
        }
        if out.len() >= lim.max {
            truncated = true;
            break;
        }
        out.push(Entry { name, dir: e.file_type().is_some_and(|t| t.is_dir()) });
        if out.len() - sent >= lim.chunk {
            on_chunk(&out[sent..]);
            sent = out.len();
        }
    }
    if out.len() > sent {
        on_chunk(&out[sent..]);
    }
    out.sort_by(|a, b| b.dir.cmp(&a.dir).then(a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    let total = out.len();
    let (offset, limit) = page;
    let entries = out.into_iter().skip(offset).take(limit.unwrap_or(usize::MAX)).collect();
    Ok(Listing { entries, total, truncated })
}

/// Cancel flags of the listings in flight, by the page's request id. A cancel that arrives before its listing starts
/// is remembered (bounded) so the listing ends at once.
struct Lists {
    live: HashMap<u64, Arc<AtomicBool>>,
    early: VecDeque<u64>,
}

fn lists() -> std::sync::MutexGuard<'static, Lists> {
    static LISTS: OnceLock<Mutex<Lists>> = OnceLock::new();
    LISTS.get_or_init(|| Mutex::new(Lists { live: HashMap::new(), early: VecDeque::new() })).lock().unwrap_or_else(|p| p.into_inner())
}

/// The flag of request `req` (an older one with the same id is cancelled).
fn begin(req: u64) -> Arc<AtomicBool> {
    let mut l = lists();
    let flag = Arc::new(AtomicBool::new(false));
    if let Some(i) = l.early.iter().position(|r| *r == req) {
        l.early.remove(i);
        flag.store(true, Ordering::SeqCst);
    }
    if let Some(old) = l.live.insert(req, flag.clone()) {
        old.store(true, Ordering::SeqCst);
    }
    flag
}

fn end(req: u64, flag: &Arc<AtomicBool>) {
    let mut l = lists();
    if l.live.get(&req).is_some_and(|f| Arc::ptr_eq(f, flag)) {
        l.live.remove(&req);
    }
}

fn cancel_req(req: u64) {
    let mut l = lists();
    match l.live.get(&req) {
        Some(f) => f.store(true, Ordering::SeqCst),
        None => {
            if l.early.len() >= 64 {
                l.early.pop_front();
            }
            l.early.push_back(req);
        }
    }
}

// ---------------------------------------------------------------- stat and read

fn is_code(e: &anyhow::Error, code: &str) -> bool {
    axelstra_core::msg::find(e).is_some_and(|c| c.code == code)
}

fn too_big() -> anyhow::Error {
    coded!("localfs.too_big", "file is larger than {mb} MB", mb = fsops::MAX_FILE_BYTES >> 20)
}

/// Content hash of a file (the editor's change detection); `None` when it does not exist.
pub fn stat_hash(roots: &[PathBuf], worktree: &str, rel: &str) -> Result<Option<String>> {
    let at = match locate(roots, worktree, rel) {
        Ok(a) => a,
        Err(e) if is_code(&e, "localfs.not_found") => return Ok(None),
        Err(e) => return Err(e),
    };
    let md = match std::fs::metadata(&at.full) {
        Ok(m) => m,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(io_err(&e, rel)),
    };
    if !md.is_file() {
        return Err(coded!("fs.not_file", "not a file"));
    }
    if md.len() > fsops::MAX_FILE_BYTES {
        return Err(too_big());
    }
    match std::fs::read(&at.full) {
        Ok(b) => Ok(Some(fsops::hash(&b))),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
        Err(e) => Err(io_err(&e, rel)),
    }
}

/// A UTF-8 text file up to `fsops::MAX_FILE_BYTES`, in the shape of the daemon's `fs.read`.
pub fn read_text(roots: &[PathBuf], worktree: &str, rel: &str) -> Result<Opened> {
    let at = locate(roots, worktree, rel)?;
    let md = std::fs::metadata(&at.full).map_err(|e| io_err(&e, rel))?;
    if !md.is_file() {
        return Err(coded!("fs.not_file", "not a file"));
    }
    if md.len() > fsops::MAX_FILE_BYTES {
        return Err(too_big());
    }
    let bytes = std::fs::read(&at.full).map_err(|e| io_err(&e, rel))?;
    if bytes.contains(&0) {
        return Err(coded!("fs.binary", "binary file"));
    }
    let hash = fsops::hash(&bytes);
    let text = String::from_utf8(bytes).map_err(|_| coded!("fs.not_utf8", "not valid UTF-8 text"))?;
    let (text, bom) = match text.strip_prefix('\u{feff}') {
        Some(t) => (t.to_string(), true),
        None => (text, false),
    };
    let crlf = text.contains("\r\n");
    Ok(Opened { text: if crlf { text.replace("\r\n", "\n") } else { text }, eol: if crlf { "crlf" } else { "lf" }, bom, hash })
}

// ---------------------------------------------------------------- open and reveal

/// Resolves and checks an open/reveal with the rules of terminal paths (`pathopen`): a program or script is never
/// opened from here (it would run), only revealed. `Run` is not offered.
pub fn plan_act(roots: &[PathBuf], worktree: &str, rel: &str, action: &str) -> Result<(PathInfo, Action)> {
    let action = match Action::parse(action) {
        Some(a @ (Action::Open | Action::Reveal)) => a,
        _ => return Err(coded!("localfs.invalid_action", "unknown action: {action}", action = action)),
    };
    let at = locate(roots, worktree, rel)?;
    let info = pathopen::resolve(&at.full.to_string_lossy(), None, std::slice::from_ref(&at.root))?;
    if !pathopen::allowed(&info, action) {
        return Err(coded!("files.path_not_allowed", "not allowed to {action} {path}", action = action.name(), path = shown(rel)));
    }
    Ok((info, action))
}

// ---------------------------------------------------------------- commands

fn store() -> PathBuf {
    axelstra_daemon::projects::store_path()
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T, CallError> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(|e| CallError::from(e.to_string()))?.map_err(|e| CallError::from(&e))
}

/// One folder of a local worktree. Streams `on_chunk` (raw entries, disk order) while reading, answers sorted.
#[tauri::command]
pub async fn local_fs_list(
    req: u64,
    worktree: String,
    path: String,
    offset: Option<usize>,
    limit: Option<usize>,
    on_chunk: Channel<Vec<Entry>>,
) -> Result<Listing, CallError> {
    let flag = begin(req);
    let f2 = flag.clone();
    let r = blocking(move || {
        list(&roots(&store()), &worktree, &path, &f2, (offset.unwrap_or(0), limit), &LIMITS, |c| {
            let _ = on_chunk.send(c.iter().map(|e| Entry { name: e.name.clone(), dir: e.dir }).collect());
        })
    })
    .await;
    end(req, &flag);
    r
}

/// Stops the listing `req` (navigating away). Unknown or finished ids are fine.
#[tauri::command]
pub fn local_fs_cancel(req: u64) {
    cancel_req(req);
}

#[tauri::command]
pub async fn local_fs_stat(worktree: String, path: String) -> Result<Option<String>, CallError> {
    blocking(move || stat_hash(&roots(&store()), &worktree, &path)).await
}

#[tauri::command]
pub async fn local_fs_read(worktree: String, path: String) -> Result<Opened, CallError> {
    blocking(move || read_text(&roots(&store()), &worktree, &path)).await
}

/// `action`: `"open"` (default program, regular non-program files only) or `"reveal"` (file manager).
#[tauri::command]
pub async fn local_fs_act(worktree: String, path: String, action: String) -> Result<(), CallError> {
    blocking(move || {
        let (info, action) = plan_act(&roots(&store()), &worktree, &path, &action)?;
        pathopen::perform(&info, action)
    })
    .await
}

static WATCH: Mutex<Option<Arc<AtomicBool>>> = Mutex::new(None);

/// Watches one local worktree (a new call replaces the old watch, `None` stops it) and emits the event
/// `local-fs-changed` `{worktree}` once per debounced batch. Event-driven (the daemon's watcher, in this process).
#[tauri::command]
pub async fn local_fs_watch(app: tauri::AppHandle, worktree: Option<String>) -> Result<(), CallError> {
    use tauri::Emitter;
    let stop = Arc::new(AtomicBool::new(false));
    let old = WATCH.lock().unwrap_or_else(|p| p.into_inner()).take();
    if let Some(old) = old {
        old.store(true, Ordering::SeqCst);
        axelstra_daemon::watcher::wake_stopped();
    }
    let Some(worktree) = worktree else { return Ok(()) };
    let s2 = stop.clone();
    blocking(move || {
        let at = locate(&roots(&store()), &worktree, "")?;
        axelstra_daemon::watcher::start(at.root, s2, move || {
            let _ = app.emit("local-fs-changed", serde_json::json!({ "worktree": worktree }));
        })
    })
    .await?;
    *WATCH.lock().unwrap_or_else(|p| p.into_inner()) = Some(stop);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU32;

    /// A scratch folder under the temp dir, removed on drop. Never the real data dir.
    struct Tmp(PathBuf);
    impl Tmp {
        fn new() -> Tmp {
            static N: AtomicU32 = AtomicU32::new(0);
            let d = std::env::temp_dir().join(format!("ax-localfs-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
            std::fs::create_dir_all(&d).unwrap();
            Tmp(axelstra_core::git::canon(&d).unwrap())
        }
        fn dir(&self, rel: &str) -> PathBuf {
            let p = self.0.join(rel);
            std::fs::create_dir_all(&p).unwrap();
            p
        }
        fn file(&self, rel: &str, body: &[u8]) -> PathBuf {
            let p = self.0.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, body).unwrap();
            p
        }
    }
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn code<T>(r: Result<T>) -> String {
        match r {
            Ok(_) => panic!("expected an error"),
            Err(e) => axelstra_core::msg::find(&e).map(|c| c.code.clone()).unwrap_or_default(),
        }
    }

    fn s(p: &Path) -> String {
        p.to_string_lossy().into_owned()
    }

    fn no_cancel() -> AtomicBool {
        AtomicBool::new(false)
    }

    fn ls(roots: &[PathBuf], wt: &Path, rel: &str) -> Result<Listing> {
        list(roots, &s(wt), rel, &no_cancel(), (0, None), &LIMITS, |_| {})
    }

    #[test]
    fn roots_come_from_projects_json_and_linked_worktrees() {
        let t = Tmp::new();
        let proj = t.dir("proj");
        let wt = t.dir("wt1");
        let admin = t.dir("proj/.git/worktrees/wt1");
        std::fs::write(admin.join("gitdir"), format!("{}\n", wt.join(".git").display())).unwrap();
        let store = t.0.join("projects.json");
        std::fs::write(&store, format!(r#"{{"v":1,"data":[{{"id":"a","name":"p","path":{:?}}}]}}"#, s(&proj))).unwrap();
        let r = roots(&store);
        assert!(r.contains(&proj) && r.contains(&wt), "{r:?}");
        // an unreadable store falls back to a backup, and to nothing
        std::fs::write(&store, "garbage").unwrap();
        std::fs::write(t.0.join("projects.json.1"), format!(r#"[{{"path":{:?}}}]"#, s(&proj))).unwrap();
        assert!(roots(&store).contains(&proj));
        assert!(roots(&t.0.join("nothing.json")).is_empty());
    }

    #[test]
    fn lists_folders_first_and_hides_git() {
        let t = Tmp::new();
        t.file("b.txt", b"x");
        t.file("A.txt", b"x");
        t.dir("zdir");
        t.dir(".git");
        t.file("zdir/inner.txt", b"x");
        let roots = vec![t.0.clone()];
        let l = ls(&roots, &t.0, "").unwrap();
        let names: Vec<_> = l.entries.iter().map(|e| (e.name.as_str(), e.dir)).collect();
        assert_eq!(names, [("zdir", true), ("A.txt", false), ("b.txt", false)]);
        assert_eq!((l.total, l.truncated), (3, false));
        assert_eq!(ls(&roots, &t.0, "zdir").unwrap().entries[0].name, "inner.txt");
        assert_eq!(code(ls(&roots, &t.0, "b.txt")), "localfs.not_dir");
        assert_eq!(code(ls(&roots, &t.0, "gone")), "localfs.not_found");
    }

    #[test]
    fn traversal_and_absolute_paths_are_refused() {
        let t = Tmp::new();
        let outside = t.dir("outside");
        let root = t.dir("root");
        t.file("outside/secret.txt", b"s");
        let roots = vec![root.clone()];
        for bad in ["..", "../outside", "a/../../outside", "sub/..\\..", &s(&outside), "/etc", "x\0y"] {
            let c = code(locate(&roots, &s(&root), bad));
            assert!(c == "localfs.invalid_path" || c == "localfs.not_found", "{bad}: {c}");
        }
        for bad in ["..", "../outside", &s(&outside), "/etc"] {
            assert_eq!(code(locate(&roots, &s(&root), bad)), "localfs.invalid_path", "{bad}");
        }
        assert_eq!(code(read_text(&roots, &s(&root), "../outside/secret.txt")), "localfs.invalid_path");
    }

    #[test]
    fn only_registered_roots_are_served() {
        let t = Tmp::new();
        let a = t.dir("a");
        let b = t.dir("b");
        let roots = vec![a.clone()];
        assert!(locate(&roots, &s(&a), "").is_ok());
        assert_eq!(code(locate(&roots, &s(&b), "")), "localfs.outside_roots");
        // a subfolder of a root is not a root: the page names the root and a relative path
        t.dir("a/sub");
        assert_eq!(code(locate(&roots, &s(&a.join("sub")), "")), "localfs.outside_roots");
        assert_eq!(code(locate(&roots, &s(t.0.parent().unwrap()), "")), "localfs.outside_roots");
        assert_eq!(code(locate(&[], &s(&a), "")), "localfs.outside_roots");
        // the page's spelling (slashes, trailing slash) still names the same root
        assert!(locate(&roots, &format!("{}/", s(&a).replace('\\', "/")), "").is_ok());
    }

    #[test]
    fn remote_unc_and_device_paths_are_not_local() {
        let roots = vec![PathBuf::from("/x")];
        assert_eq!(code(locate(&roots, "@lab:/home/me", "")), "localfs.not_local");
        assert_eq!(code(locate(&roots, "", "")), "localfs.not_local");
        for p in [r"\\server\share\x", "//server/share", r"\\.\C:\x", r"\\?\UNC\server\share", r"\\?\GLOBALROOT\Device\x"] {
            assert!(is_unc_or_device(p), "{p}");
        }
        for p in [r"\\?\C:\work", r"C:\work", "/home/me", "rel/x", ""] {
            assert!(!is_unc_or_device(p), "{p}");
        }
    }

    #[test]
    fn windows_reserved_names() {
        for n in ["CON", "con", "NUL", "nul.txt", "Aux.tar.gz", "PRN", "COM1", "com9.log", "LPT3", "CONIN$", "CON  ", "COM¹"] {
            assert!(is_reserved(n), "{n}");
        }
        for n in ["COM0", "COM10", "LPT", "CONSOLE", "conf", "nulx", "a.con", "com", "", "src"] {
            assert!(!is_reserved(n), "{n}");
        }
        let t = Tmp::new();
        let roots = vec![t.0.clone()];
        let c = code(locate(&roots, &s(&t.0), "dir/NUL.txt"));
        // refused as a name on Windows; elsewhere such a file is ordinary and simply absent
        assert_eq!(c, if cfg!(windows) { "localfs.reserved_name" } else { "localfs.not_found" });
        if cfg!(windows) {
            for bad in ["a:stream", "name.", "name ", "C:\\x"] {
                assert!(check_rel(bad).is_err(), "{bad}");
            }
        }
    }

    #[test]
    fn a_symlink_leaving_the_root_is_refused() {
        let t = Tmp::new();
        let outside = t.dir("outside");
        t.file("outside/secret.txt", b"s");
        let root = t.dir("root");
        #[cfg(unix)]
        let made = std::os::unix::fs::symlink(&outside, root.join("link")).is_ok();
        #[cfg(windows)]
        let made = std::os::windows::fs::symlink_dir(&outside, root.join("link")).is_ok();
        if !made {
            eprintln!("skipped: no privilege to create symlinks");
            return;
        }
        let roots = vec![root.clone()];
        assert_eq!(code(locate(&roots, &s(&root), "link")), "localfs.outside_roots");
        assert_eq!(code(read_text(&roots, &s(&root), "link/secret.txt")), "localfs.outside_roots");
        assert_eq!(code(ls(&roots, &root, "link")), "localfs.outside_roots");
        assert_eq!(code(stat_hash(&roots, &s(&root), "link/secret.txt")), "localfs.outside_roots");
        // the link itself is listed (as a plain entry) but never followed
        assert!(ls(&roots, &root, "").unwrap().entries.iter().any(|e| e.name == "link"));
    }

    #[test]
    fn big_folder_is_capped_streamed_in_chunks_and_paged() {
        let t = Tmp::new();
        for i in 0..130 {
            t.file(&format!("big/f{i:03}.txt"), b"");
        }
        let roots = vec![t.0.clone()];
        let mut sizes = vec![];
        let lim = Limits { max: 100, chunk: 40 };
        let l = list(&roots, &s(&t.0), "big", &no_cancel(), (0, None), &lim, |c| sizes.push(c.len())).unwrap();
        assert_eq!((l.total, l.truncated, l.entries.len()), (100, true, 100));
        assert_eq!(sizes, [40, 40, 20]);
        // paging over the sorted result
        let l = list(&roots, &s(&t.0), "big", &no_cancel(), (10, Some(5)), &Limits { max: 1000, chunk: 1000 }, |_| {}).unwrap();
        assert_eq!((l.total, l.truncated), (130, false));
        let names: Vec<_> = l.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["f010.txt", "f011.txt", "f012.txt", "f013.txt", "f014.txt"]);
    }

    #[test]
    fn cancel_stops_a_listing_even_before_it_starts() {
        let t = Tmp::new();
        t.file("a.txt", b"");
        let roots = vec![t.0.clone()];
        let flag = AtomicBool::new(true);
        let r = list(&roots, &s(&t.0), "", &flag, (0, None), &LIMITS, |_| {});
        assert_eq!(code(r), "localfs.cancelled");
        // by id: a cancel that came first, then one while running
        cancel_req(990_001);
        let f = begin(990_001);
        assert!(f.load(Ordering::SeqCst));
        end(990_001, &f);
        let f = begin(990_002);
        assert!(!f.load(Ordering::SeqCst));
        cancel_req(990_002);
        assert!(f.load(Ordering::SeqCst));
        // a newer request with the same id cancels the older one
        let g = begin(990_002);
        assert!(f.load(Ordering::SeqCst) && !g.load(Ordering::SeqCst));
        end(990_002, &f);
        assert!(lists().live.contains_key(&990_002), "a stale end must not drop the newer flag");
        end(990_002, &g);
        assert!(!lists().live.contains_key(&990_002));
    }

    #[test]
    fn reads_text_and_stats_with_the_daemons_shapes() {
        let t = Tmp::new();
        t.file("a.txt", b"\xef\xbb\xbfone\r\ntwo\r\n");
        t.file("bin.dat", b"a\0b");
        t.file("latin.txt", b"\xe9");
        t.file("huge.txt", &vec![b'x'; (fsops::MAX_FILE_BYTES + 1) as usize]);
        t.dir("d");
        let roots = vec![t.0.clone()];
        let w = s(&t.0);
        let o = read_text(&roots, &w, "a.txt").unwrap();
        assert_eq!((o.text.as_str(), o.eol, o.bom), ("one\ntwo\n", "crlf", true));
        assert_eq!(o.hash, fsops::hash(b"\xef\xbb\xbfone\r\ntwo\r\n"));
        assert_eq!(stat_hash(&roots, &w, "a.txt").unwrap(), Some(o.hash));
        assert_eq!(stat_hash(&roots, &w, "missing.txt").unwrap(), None);
        assert_eq!(code(read_text(&roots, &w, "bin.dat")), "fs.binary");
        assert_eq!(code(read_text(&roots, &w, "latin.txt")), "fs.not_utf8");
        assert_eq!(code(read_text(&roots, &w, "huge.txt")), "localfs.too_big");
        assert_eq!(code(read_text(&roots, &w, "d")), "fs.not_file");
        assert_eq!(code(read_text(&roots, &w, "missing.txt")), "localfs.not_found");
    }

    #[test]
    fn open_never_runs_programs_and_reveal_stays_inside() {
        let t = Tmp::new();
        t.file("notes.txt", b"x");
        t.file("tool.exe", b"x");
        t.dir("d");
        let roots = vec![t.0.clone()];
        let w = s(&t.0);
        assert!(plan_act(&roots, &w, "notes.txt", "open").is_ok());
        assert!(plan_act(&roots, &w, "notes.txt", "reveal").is_ok());
        assert!(plan_act(&roots, &w, "d", "reveal").is_ok());
        assert_eq!(code(plan_act(&roots, &w, "tool.exe", "open")), "files.path_not_allowed");
        assert!(plan_act(&roots, &w, "tool.exe", "reveal").is_ok());
        assert_eq!(code(plan_act(&roots, &w, "d", "open")), "files.path_not_allowed");
        assert_eq!(code(plan_act(&roots, &w, "tool.exe", "run")), "localfs.invalid_action");
        assert_eq!(code(plan_act(&roots, &w, "../x", "reveal")), "localfs.invalid_path");
        assert_eq!(code(plan_act(&roots, &w, "gone.txt", "open")), "localfs.not_found");
    }
}
