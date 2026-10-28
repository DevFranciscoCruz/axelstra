//! Keeps antivirus and file indexers off the worktree folders (`<repo parent>/.axelstra-wt`): node_modules and target
//! trees of a score of worktrees made Defender (MsMpEng) and Windows Search (SearchIndexer) eat the CPU.
//!
//! Two parts, both behind traits so tests never touch the machine:
//!
//! * **Automatic** (no admin, silent, idempotent, reversible by hand): Windows gets `FILE_ATTRIBUTE_NOT_CONTENT_INDEXED` on
//!   the `.axelstra-wt` folder and its worktrees (NTFS gives it to everything created below a flagged folder, so only
//!   existing content needs a walk, in a low-priority thread with a cap); macOS gets an empty `.metadata_never_index`;
//!   Linux gets nothing (documented in `docs/SCANX.md`). Triggered by events: a worktree or run creating the folder and
//!   the daemon starting. No polling.
//! * **With consent** (Windows, admin): a Defender path exclusion for exactly those folders, added or taken back by an
//!   elevated PowerShell the user confirms with UAC (`Fs`/`Runner` below; `server/scanx.rs` has the RPCs). Nothing here
//!   ever applies it by itself.
//!
//! A non-admin process cannot read Defender's exclusions (`Get-MpPreference` answers `N/A: Must be an administrator to view
//! exclusions`), so the state is "unknown" unless the exclusions are readable or we recorded having added the path.
//!
//! Not done on purpose: a Windows Search crawl-scope rule (`ISearchCrawlScopeManager`). It needs COM interop that no
//! dependency of the workspace provides and cannot be proven without admin on a real machine; the content-indexed
//! attribute already keeps the indexer from reading file contents.

use anyhow::Result;
use axelstra_core::coded;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Name of the folder that holds the worktrees.
pub const WT_DIR: &str = ".axelstra-wt";
/// macOS: Spotlight skips a folder that holds this file.
pub const MARKER: &str = ".metadata_never_index";
pub const NOT_CONTENT_INDEXED: u32 = 0x2000;
const ATTR_NORMAL: u32 = 0x80;
const ATTR_REPARSE: u32 = 0x400;
/// Attribute changes made for one tree before the walk stops (it goes on at the next start).
pub const MAX_SET: usize = 50_000;
/// Entries looked at for one tree before the walk stops.
pub const MAX_VISIT: usize = 1_000_000;
/// Paths one elevated command line carries (the command line is limited to about 32 000 characters).
const MAX_PATHS: usize = 40;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Os {
    Windows,
    MacOs,
    Other,
}

impl Os {
    pub fn current() -> Os {
        if cfg!(windows) {
            Os::Windows
        } else if cfg!(target_os = "macos") {
            Os::MacOs
        } else {
            Os::Other
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Os::Windows => "windows",
            Os::MacOs => "macos",
            Os::Other => "linux",
        }
    }
}

// ---------------------------------------------------------------- file system

pub struct Entry {
    pub path: PathBuf,
    pub is_dir: bool,
    /// symlink, junction or other reparse point: never followed, never changed
    pub is_link: bool,
}

/// The file system calls the automatic part needs.
pub trait Fs: Send + Sync {
    fn attrs(&self, p: &Path) -> io::Result<u32>;
    fn set_attrs(&self, p: &Path, a: u32) -> io::Result<()>;
    fn is_dir(&self, p: &Path) -> bool;
    fn children(&self, p: &Path) -> io::Result<Vec<Entry>>;
    /// Creates `p` as an empty file when it does not exist.
    fn touch(&self, p: &Path) -> io::Result<()>;
    fn exists(&self, p: &Path) -> bool;
}

/// The only unsafe code of the module: Win32 attribute calls and the thread priority.
#[cfg(windows)]
mod sys {
    use std::io;
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;
    use windows_sys::Win32::Storage::FileSystem::{GetFileAttributesW, SetFileAttributesW, INVALID_FILE_ATTRIBUTES};
    use windows_sys::Win32::System::Threading::{GetCurrentThread, SetThreadPriority, THREAD_MODE_BACKGROUND_BEGIN};

    fn wide(p: &Path) -> Vec<u16> {
        let s = p.as_os_str().to_string_lossy();
        // long absolute paths (node_modules) only work with the verbatim prefix
        let long = s.len() > 240 && !s.starts_with(r"\\") && s.as_bytes().get(1) == Some(&b':');
        let full = if long { format!(r"\\?\{s}") } else { s.into_owned() };
        std::ffi::OsStr::new(&full).encode_wide().chain(std::iter::once(0)).collect()
    }

    pub fn get_attrs(p: &Path) -> io::Result<u32> {
        let w = wide(p);
        // SAFETY: `w` is a NUL-terminated UTF-16 string that outlives the call.
        let a = unsafe { GetFileAttributesW(w.as_ptr()) };
        if a == INVALID_FILE_ATTRIBUTES {
            Err(io::Error::last_os_error())
        } else {
            Ok(a)
        }
    }

    pub fn set_attrs(p: &Path, attrs: u32) -> io::Result<()> {
        let w = wide(p);
        // SAFETY: as above.
        if unsafe { SetFileAttributesW(w.as_ptr(), attrs) } == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    /// Lowers the CPU, I/O and memory priority of the calling thread (it ends with the thread).
    pub fn background_thread() {
        // SAFETY: acts on the current thread only; a failure leaves the priority as it was.
        unsafe {
            SetThreadPriority(GetCurrentThread(), THREAD_MODE_BACKGROUND_BEGIN);
        }
    }
}

pub struct RealFs;

impl Fs for RealFs {
    fn attrs(&self, p: &Path) -> io::Result<u32> {
        #[cfg(windows)]
        {
            sys::get_attrs(p)
        }
        #[cfg(not(windows))]
        {
            let _ = p;
            Err(io::Error::from(io::ErrorKind::Unsupported))
        }
    }

    fn set_attrs(&self, p: &Path, a: u32) -> io::Result<()> {
        #[cfg(windows)]
        {
            sys::set_attrs(p, a)
        }
        #[cfg(not(windows))]
        {
            let _ = (p, a);
            Err(io::Error::from(io::ErrorKind::Unsupported))
        }
    }

    fn is_dir(&self, p: &Path) -> bool {
        p.is_dir()
    }

    fn children(&self, p: &Path) -> io::Result<Vec<Entry>> {
        let mut out = vec![];
        for e in std::fs::read_dir(p)?.flatten() {
            let Ok(md) = e.path().symlink_metadata() else { continue };
            #[cfg(windows)]
            let reparse = std::os::windows::fs::MetadataExt::file_attributes(&md) & ATTR_REPARSE != 0;
            #[cfg(not(windows))]
            let reparse = false;
            out.push(Entry { path: e.path(), is_dir: md.is_dir(), is_link: md.file_type().is_symlink() || reparse });
        }
        Ok(out)
    }

    fn touch(&self, p: &Path) -> io::Result<()> {
        std::fs::OpenOptions::new().create(true).append(true).open(p).map(|_| ())
    }

    fn exists(&self, p: &Path) -> bool {
        p.exists()
    }
}

/// Lowers the priority of the calling thread (Windows); elsewhere nothing.
pub fn background_thread() {
    #[cfg(windows)]
    sys::background_thread();
}

/// The attributes with the content-indexed flag off. `NORMAL` is valid only alone, so it goes when another bit is set.
pub fn with_flag(attrs: u32) -> u32 {
    (attrs & !ATTR_NORMAL) | NOT_CONTENT_INDEXED
}

#[derive(Default, Debug, PartialEq, Clone)]
pub struct Applied {
    /// items that got the flag (or the marker file)
    pub changed: usize,
    /// items that already had it
    pub already: usize,
    pub failed: usize,
    /// the walk stopped at a cap: the tree is not finished
    pub capped: bool,
}

impl Applied {
    fn add(&mut self, o: &Applied) {
        self.changed += o.changed;
        self.already += o.already;
        self.failed += o.failed;
        self.capped |= o.capped;
    }
}

/// Sets the flag on one item, keeping its other attributes. Returns whether it was changed (`Ok(false)`: already set).
fn flag_item(fs: &dyn Fs, p: &Path) -> io::Result<bool> {
    let a = fs.attrs(p)?;
    if a & NOT_CONTENT_INDEXED != 0 {
        return Ok(false);
    }
    fs.set_attrs(p, with_flag(a))?;
    Ok(true)
}

fn count(out: &mut Applied, r: io::Result<bool>) {
    match r {
        Ok(true) => out.changed += 1,
        Ok(false) => out.already += 1,
        Err(_) => out.failed += 1,
    }
}

/// Flags every directory and file below `tree` that lacks the flag, skipping links. Stops at `max_set` changes or
/// `max_visit` entries (`capped`). The caller flags `tree` itself afterwards, so a flagged tree means a finished one.
pub fn walk_tree(fs: &dyn Fs, tree: &Path, max_set: usize, max_visit: usize) -> Applied {
    let mut out = Applied::default();
    let mut visited = 0usize;
    let mut stack = vec![tree.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(kids) = fs.children(&dir) else {
            out.failed += 1;
            continue;
        };
        for k in kids {
            if k.is_link {
                continue;
            }
            visited += 1;
            if visited > max_visit || out.changed >= max_set {
                out.capped = true;
                return out;
            }
            count(&mut out, flag_item(fs, &k.path));
            if k.is_dir {
                stack.push(k.path);
            }
        }
    }
    out
}

/// What a freshly created (or found) `.axelstra-wt` folder gets at once, before anything is put in it: the flag on the
/// folder (everything created below inherits it) or the macOS marker. Cheap enough to run inside `worktree.create`.
pub fn protect_root(fs: &dyn Fs, os: Os, root: &Path) -> Applied {
    let mut out = Applied::default();
    match os {
        Os::Windows => count(&mut out, flag_item(fs, root)),
        Os::MacOs => {
            let m = root.join(MARKER);
            if fs.exists(&m) {
                out.already += 1;
            } else {
                match fs.touch(&m) {
                    Ok(()) => out.changed += 1,
                    Err(_) => out.failed += 1,
                }
            }
        }
        Os::Other => {}
    }
    out
}

/// The whole job for one `.axelstra-wt` folder: `protect_root`, then (Windows) every worktree folder that is not flagged
/// yet gets its existing content flagged and is flagged last. A worktree made after the root was flagged is born flagged,
/// so the next start only reads one attribute per worktree.
pub fn sweep_root(fs: &dyn Fs, os: Os, root: &Path, max_set: usize, max_visit: usize) -> Applied {
    let mut out = protect_root(fs, os, root);
    if os != Os::Windows {
        return out;
    }
    let Ok(kids) = fs.children(root) else { return out };
    for k in kids.into_iter().filter(|k| k.is_dir && !k.is_link) {
        if fs.attrs(&k.path).is_ok_and(|a| a & NOT_CONTENT_INDEXED != 0) {
            out.already += 1;
            continue;
        }
        let t = walk_tree(fs, &k.path, max_set, max_visit);
        let capped = t.capped;
        out.add(&t);
        if !capped {
            count(&mut out, flag_item(fs, &k.path));
        }
    }
    out
}

/// `<repo parent>/.axelstra-wt` of a project folder.
pub fn root_of(repo: &Path) -> Option<PathBuf> {
    repo.parent().filter(|p| p.file_name().is_some()).map(|p| p.join(WT_DIR))
}

/// Flags the root with the real file system, at once (called by `worktree.create` before git fills the folder).
pub fn on_root_ready(root: &Path) {
    protect_root(&RealFs, Os::current(), root);
}

/// Roots being swept, so a burst of events does not stack threads on the same folder.
fn in_flight() -> &'static Mutex<std::collections::HashSet<String>> {
    static F: std::sync::OnceLock<Mutex<std::collections::HashSet<String>>> = std::sync::OnceLock::new();
    F.get_or_init(Default::default)
}

/// Sweeps the roots in a low-priority thread, one after the other. By event (daemon start); never holds a lock of the
/// registry or the stores.
pub fn spawn_sweep(roots: Vec<PathBuf>) {
    let os = Os::current();
    if os == Os::Other || roots.is_empty() {
        return;
    }
    let roots: Vec<PathBuf> = roots
        .into_iter()
        .filter(|r| r.is_dir())
        .filter(|r| in_flight().lock().unwrap_or_else(|e| e.into_inner()).insert(axelstra_core::paths::norm_path(r)))
        .collect();
    if roots.is_empty() {
        return;
    }
    let _ = std::thread::Builder::new().name("scanx-sweep".into()).spawn(move || {
        background_thread();
        for r in roots {
            let res = sweep_root(&RealFs, os, &r, MAX_SET, MAX_VISIT);
            if res.failed > 0 || res.capped {
                eprintln!("scanx: {} changed, {} failed{}", res.changed, res.failed, if res.capped { ", capped" } else { "" });
            }
            in_flight().lock().unwrap_or_else(|e| e.into_inner()).remove(&axelstra_core::paths::norm_path(&r));
        }
    });
}

// ----------------------------------------------------------------- PowerShell

/// `s` as a PowerShell single-quoted literal. Besides `'`, PowerShell treats the typographic quotes as quotes, so those
/// are doubled too. Control characters have no business in a path and are refused.
pub fn ps_quote(s: &str) -> Result<String> {
    if s.chars().any(|c| c.is_control()) {
        return Err(coded!("scanx.bad_path", "refusing a path with control characters: {path}", path = s.escape_debug()));
    }
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for c in s.chars() {
        out.push(c);
        if matches!(c, '\'' | '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{201B}') {
            out.push(c);
        }
    }
    out.push('\'');
    Ok(out)
}

/// `-EncodedCommand` payload: base64 of the UTF-16LE script.
pub fn encode_command(script: &str) -> String {
    use base64::Engine;
    let bytes: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Op {
    Add,
    Remove,
}

/// Key to compare two exclusion paths: separators unified, no trailing one, lowercase, no verbatim prefix.
pub fn canon(s: &str) -> String {
    let s = s.strip_prefix(r"\\?\").unwrap_or(s);
    s.replace('/', "\\").trim_end_matches('\\').to_lowercase()
}

/// True when the folder named by `entry` is `path` or contains it.
fn covers(entry: &str, path: &str) -> bool {
    let (e, p) = (canon(entry), canon(path));
    !e.is_empty() && (p == e || p.strip_prefix(&e).is_some_and(|r| r.starts_with('\\')))
}

/// A path the app may ask Defender to exclude: absolute, a `.axelstra-wt` folder, never a drive root.
pub fn check_exclusion_path(p: &str) -> Result<()> {
    let c = canon(p);
    let drive_abs = c.len() > 3 && c.as_bytes()[1] == b':' && c.as_bytes()[2] == b'\\';
    let unc = c.starts_with("\\\\");
    if !(drive_abs || unc) || c.rsplit('\\').next() != Some(WT_DIR) {
        return Err(coded!("scanx.bad_path", "refusing to exclude {path}: only .axelstra-wt folders", path = p));
    }
    Ok(())
}

/// The script run elevated. It reads the real exclusions (only an admin can), adds or removes the wanted paths when they
/// are missing or present (so running it twice changes nothing), reads them again and writes
/// `{ok, before, after, error}` as JSON to `result_path`. Paths are quoted literals; nothing else is interpolated.
pub fn elevated_script(op: Op, paths: &[String], result_path: &Path) -> Result<String> {
    let want = paths.iter().map(|p| ps_quote(p)).collect::<Result<Vec<_>>>()?.join(",");
    let out = ps_quote(&result_path.to_string_lossy())?;
    let act = match op {
        Op::Add => "if($idx -lt 0){ Add-MpPreference -ExclusionPath $w }",
        Op::Remove => "if($idx -ge 0){ Remove-MpPreference -ExclusionPath $before[$idx] }",
    };
    Ok(format!(
        "$ErrorActionPreference='Stop'\n\
         $res=[ordered]@{{ok=$false;before=@();after=@();error=$null}}\n\
         try{{\n\
         $want=@({want})\n\
         $key={{param($s) ([string]$s).Replace('/','\\').TrimEnd('\\').ToLowerInvariant()}}\n\
         $before=@((Get-MpPreference).ExclusionPath | Where-Object {{$_}})\n\
         $res.before=$before\n\
         foreach($w in $want){{\n\
         $k=& $key $w\n\
         $idx=-1\n\
         for($i=0;$i -lt $before.Count;$i++){{ if((& $key $before[$i]) -eq $k){{ $idx=$i; break }} }}\n\
         {act}\n\
         }}\n\
         $res.after=@((Get-MpPreference).ExclusionPath | Where-Object {{$_}})\n\
         $res.ok=$true\n\
         }}catch{{ $res.error=$_.Exception.Message }}\n\
         [IO.File]::WriteAllText({out},($res|ConvertTo-Json -Compress -Depth 4),(New-Object Text.UTF8Encoding $false))\n"
    ))
}

/// The script that launches `elevated_script` through UAC and waits. Exit 0: it ran; 6: the user cancelled the UAC
/// prompt (Win32 error 1223); 5: it could not be started.
pub fn launcher_script(inner: &str) -> String {
    format!(
        "try{{ Start-Process -FilePath 'powershell.exe' -Verb RunAs -Wait -WindowStyle Hidden -ArgumentList '-NoProfile','-NonInteractive','-ExecutionPolicy','Bypass','-EncodedCommand','{}'; exit 0 }}\n\
         catch{{ [Console]::Error.WriteLine($_.Exception.Message); if($_.Exception -is [System.ComponentModel.Win32Exception] -and $_.Exception.NativeErrorCode -eq 1223){{ exit 6 }}; exit 5 }}\n",
        encode_command(inner)
    )
}

/// Read-only script (no admin): Defender state, the exclusions when readable, and the file system of the drives.
pub fn status_script(drives: &[char]) -> String {
    let letters =
        drives.iter().filter(|c| c.is_ascii_alphabetic()).map(|c| format!("'{}'", c.to_ascii_uppercase())).collect::<Vec<_>>().join(",");
    format!(
        "[Console]::OutputEncoding=[Text.Encoding]::UTF8\n\
         $ErrorActionPreference='SilentlyContinue'\n\
         $o=[ordered]@{{defender=$false;av=$null;rtp=$null;tamper=$null;excl=$null;vols=[ordered]@{{}}}}\n\
         try{{ $s=Get-MpComputerStatus -ErrorAction Stop; $o.defender=$true; $o.av=[bool]$s.AntivirusEnabled; $o.rtp=[bool]$s.RealTimeProtectionEnabled; $o.tamper=[bool]$s.IsTamperProtected }}catch{{}}\n\
         try{{ $o.excl=@((Get-MpPreference -ErrorAction Stop).ExclusionPath) }}catch{{}}\n\
         foreach($d in @({letters})){{ try{{ $o.vols[$d]=[string](Get-Volume -DriveLetter $d -ErrorAction Stop).FileSystemType }}catch{{}} }}\n\
         $o|ConvertTo-Json -Compress -Depth 4\n"
    )
}

/// Read-only script: CPU and memory of the scanners right now (one sample, no loop).
pub fn procs_script() -> String {
    "[Console]::OutputEncoding=[Text.Encoding]::UTF8\n\
     $ErrorActionPreference='SilentlyContinue'\n\
     @(Get-CimInstance Win32_PerfFormattedData_PerfProc_Process -Filter \"Name='MsMpEng' OR Name='SearchIndexer' OR Name='SearchProtocolHost'\" | Select-Object Name,IDProcess,PercentProcessorTime,WorkingSet) | ConvertTo-Json -Compress\n"
        .to_string()
}

/// How the daemon runs PowerShell. Real: `PsRunner`. Tests: a fake that never touches the machine.
pub trait Runner: Send + Sync {
    /// Runs a read-only script without elevation; its stdout.
    fn query(&self, script: &str) -> Result<String>;
    /// Adds or removes the exclusions of `paths` through UAC; the JSON the elevated script wrote.
    fn elevate(&self, op: Op, paths: &[String]) -> Result<String>;
}

pub struct PsRunner;

fn powershell(script: &str, timeout: std::time::Duration) -> Result<axelstra_core::exec::Captured> {
    let mut cmd = std::process::Command::new("powershell.exe");
    cmd.args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-EncodedCommand", &encode_command(script)]);
    axelstra_core::exec::run_capped(&mut cmd, timeout, 1 << 20)
        .map_err(|e| coded!("scanx.powershell_failed", "could not run PowerShell: {detail}", detail = format!("{e:#}")))
}

fn text(b: &[u8]) -> String {
    String::from_utf8_lossy(b).trim_start_matches('\u{feff}').trim().to_string()
}

impl Runner for PsRunner {
    fn query(&self, script: &str) -> Result<String> {
        let c = powershell(script, std::time::Duration::from_secs(30))?;
        if c.timed_out || c.status.is_none_or(|s| !s.success()) {
            return Err(coded!("scanx.powershell_failed", "could not run PowerShell: {detail}", detail = text(&c.stderr)));
        }
        Ok(text(&c.stdout))
    }

    fn elevate(&self, op: Op, paths: &[String]) -> Result<String> {
        let mut id = [0u8; 8];
        let _ = getrandom::getrandom(&mut id);
        let result =
            std::env::temp_dir().join(format!("axelstra-scanx-{}.json", id.iter().map(|b| format!("{b:02x}")).collect::<String>()));
        let inner = elevated_script(op, paths, &result)?;
        let launcher = launcher_script(&inner);
        if launcher.len() > 30_000 {
            return Err(coded!("scanx.too_many_paths", "too many folders for one elevated command ({count})", count = paths.len()));
        }
        // the user answers a UAC prompt: give them time
        let c = powershell(&launcher, std::time::Duration::from_secs(300));
        let json = std::fs::read(&result).ok().map(|b| text(&b));
        let _ = std::fs::remove_file(&result);
        let c = c?;
        match c.status.and_then(|s| s.code()) {
            Some(0) => {}
            Some(6) => return Err(coded!("scanx.elevation_cancelled", "the administrator prompt was cancelled")),
            _ => {
                let why = if c.timed_out { "timed out waiting for the administrator prompt".to_string() } else { text(&c.stderr) };
                return Err(coded!("scanx.elevation_failed", "could not start the elevated PowerShell: {detail}", detail = why));
            }
        }
        json.filter(|j| !j.is_empty()).ok_or_else(|| {
            coded!("scanx.defender_failed", "Defender did not answer: {detail}", detail = "the elevated command left no result")
        })
    }
}

// ------------------------------------------------------------------ parsing

/// A JSON value that PowerShell wrote as null, one string or an array of strings.
fn strings(v: &Value) -> Vec<String> {
    match v {
        Value::String(s) => vec![s.clone()],
        Value::Array(a) => a.iter().filter_map(|x| x.as_str().map(String::from)).collect(),
        _ => vec![],
    }
}

fn json_of(raw: &str) -> Option<Value> {
    serde_json::from_str(raw.trim_start_matches('\u{feff}').trim()).ok()
}

/// What `status_script` found.
#[derive(Debug, Default, PartialEq)]
pub struct Query {
    /// the Defender cmdlets answered
    pub defender: bool,
    pub antivirus: Option<bool>,
    pub realtime: Option<bool>,
    pub tamper: Option<bool>,
    /// the exclusions, or `None` when this process may not read them
    pub exclusions: Option<Vec<String>>,
    /// drive letter -> file system name
    pub vols: BTreeMap<char, String>,
}

impl Query {
    /// Defender is the antivirus doing real-time scanning (not switched off, not replaced by another product).
    pub fn active(&self) -> bool {
        self.defender && self.antivirus == Some(true) && self.realtime == Some(true)
    }
}

pub fn parse_query(raw: &str) -> Query {
    let Some(v) = json_of(raw) else { return Query::default() };
    let excl = strings(&v["excl"]);
    // an unprivileged read answers one line: "N/A: Must be an administrator to view exclusions"
    let readable = !v["excl"].is_null() && !excl.iter().any(|e| e.starts_with("N/A"));
    Query {
        defender: v["defender"].as_bool() == Some(true),
        antivirus: v["av"].as_bool(),
        realtime: v["rtp"].as_bool(),
        tamper: v["tamper"].as_bool(),
        exclusions: readable.then_some(excl),
        vols: v["vols"]
            .as_object()
            .map(|o| o.iter().filter_map(|(k, f)| Some((k.chars().next()?.to_ascii_uppercase(), f.as_str()?.to_string()))).collect())
            .unwrap_or_default(),
    }
}

/// What the elevated script wrote.
#[derive(Debug, PartialEq)]
pub struct Elev {
    pub before: Vec<String>,
    pub after: Vec<String>,
}

fn blocked(msg: &str) -> bool {
    let m = msg.to_lowercase();
    ["tamper", "0x800106ba", "policy", "managed", "not allowed", "denied", "blocked", "administrator"].iter().any(|k| m.contains(k))
}

pub fn parse_elev(raw: &str) -> Result<Elev> {
    let v =
        json_of(raw).ok_or_else(|| coded!("scanx.defender_failed", "Defender did not answer: {detail}", detail = "unreadable result"))?;
    if v["ok"].as_bool() != Some(true) {
        let msg = v["error"].as_str().unwrap_or("unknown error").to_string();
        return Err(if blocked(&msg) {
            coded!("scanx.defender_blocked", "Defender refused the change (Tamper Protection or a policy): {detail}", detail = msg)
        } else {
            coded!("scanx.defender_failed", "Defender did not answer: {detail}", detail = msg)
        });
    }
    Ok(Elev { before: strings(&v["before"]), after: strings(&v["after"]) })
}

#[derive(Debug, PartialEq, Default)]
pub struct Change {
    /// paths this run added or removed
    pub changed: Vec<String>,
    /// paths that were already as asked
    pub unchanged: Vec<String>,
}

/// Runs `op` for `paths` and checks the result: after an add every path must be excluded, after a remove none may be.
/// A change Defender ignored without an error is Tamper Protection or a policy.
pub fn run_change(r: &dyn Runner, op: Op, paths: &[String]) -> Result<Change> {
    if paths.is_empty() {
        return Err(coded!("scanx.no_folders", "there is no .axelstra-wt folder to change"));
    }
    if paths.len() > MAX_PATHS {
        return Err(coded!("scanx.too_many_paths", "too many folders for one elevated command ({count})", count = paths.len()));
    }
    for p in paths {
        check_exclusion_path(p)?;
    }
    let e = parse_elev(&r.elevate(op, paths)?)?;
    let mut out = Change::default();
    for p in paths {
        let (was, is) = (e.before.iter().any(|x| canon(x) == canon(p)), e.after.iter().any(|x| canon(x) == canon(p)));
        match op {
            Op::Add if !is => return Err(blocked_after(p)),
            Op::Remove if is => return Err(blocked_after(p)),
            Op::Add if was => out.unchanged.push(p.clone()),
            Op::Remove if !was => out.unchanged.push(p.clone()),
            _ => out.changed.push(p.clone()),
        }
    }
    Ok(out)
}

fn blocked_after(p: &str) -> anyhow::Error {
    coded!(
        "scanx.defender_blocked",
        "Defender refused the change (Tamper Protection or a policy): {detail}",
        detail = format!("{p} is unchanged")
    )
}

/// The recorded list after a change: `op` Add puts the paths in, Remove takes them out.
pub fn recorded_after(recorded: &[String], op: Op, paths: &[String]) -> Vec<String> {
    let mut out: Vec<String> = recorded.iter().filter(|r| !paths.iter().any(|p| canon(p) == canon(r))).cloned().collect();
    if op == Op::Add {
        out.extend(paths.iter().cloned());
    }
    out
}

/// One process of `procs_script`.
pub fn parse_procs(raw: &str) -> Vec<Value> {
    let Some(v) = json_of(raw) else { return vec![] };
    let items = match v {
        Value::Array(a) => a,
        Value::Object(_) => vec![v],
        _ => vec![],
    };
    items
        .iter()
        .filter_map(|p| {
            Some(json!({"name": p["Name"].as_str()?, "pid": p["IDProcess"], "cpu": p["PercentProcessorTime"], "rss": p["WorkingSet"]}))
        })
        .collect()
}

// -------------------------------------------------------------------- status

/// A `.axelstra-wt` folder of a registered project.
#[derive(Clone, Debug, PartialEq)]
pub struct Folder {
    pub project: String,
    pub name: String,
    pub path: String,
}

/// The folder as a Windows path (what Defender stores): backslashes, no verbatim prefix.
pub fn display_path(p: &Path) -> String {
    let s = p.to_string_lossy();
    let s = s.strip_prefix(r"\\?\").unwrap_or(&s);
    if cfg!(windows) {
        s.replace('/', "\\")
    } else {
        s.to_string()
    }
}

pub fn folders_of(projects: &[(String, String, String)]) -> Vec<Folder> {
    projects
        .iter()
        .filter_map(|(id, name, path)| {
            Some(Folder { project: id.clone(), name: name.clone(), path: display_path(&root_of(Path::new(path))?) })
        })
        .collect()
}

/// The distinct folder paths (several projects can share a parent folder).
pub fn unique_paths(folders: &[Folder]) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    folders.iter().filter(|f| seen.insert(canon(&f.path))).map(|f| f.path.clone()).collect()
}

fn drive_of(path: &str) -> Option<char> {
    let b = path.as_bytes();
    (b.len() > 2 && b[1] == b':' && b[0].is_ascii_alphabetic()).then(|| b[0].to_ascii_uppercase() as char)
}

pub fn drives_of(folders: &[Folder]) -> Vec<char> {
    let mut v: Vec<char> = folders.iter().filter_map(|f| drive_of(&f.path)).collect();
    v.sort_unstable();
    v.dedup();
    v
}

/// ReFS is what a Dev Drive is made of, and the only volume where Defender's performance mode applies. A plain ReFS
/// volume is treated alike: either way the scan cost is already low and the notice would not help.
fn is_dev_drive(fs: Option<&str>) -> bool {
    fs.is_some_and(|f| f.eq_ignore_ascii_case("ReFS"))
}

/// Persistent bits kept in `settings.json` under `scanx`.
#[derive(Default, Debug, PartialEq)]
pub struct Saved {
    /// the paths this app added to Defender (the only record, since a non-admin cannot read them)
    pub applied: Vec<String>,
    /// the notice was raised once (never again)
    pub shown: bool,
    /// the notice is waiting to be seen
    pub pending: bool,
    pub dismissed: bool,
}

impl Saved {
    pub fn from_settings(s: &Value) -> Saved {
        let v = &s["scanx"];
        Saved {
            applied: strings(&v["applied"]),
            shown: v["hint_shown"].as_bool() == Some(true),
            pending: v["hint_pending"].as_bool() == Some(true),
            dismissed: v["hint_dismissed"].as_bool() == Some(true),
        }
    }

    pub fn write_into(&self, s: &mut Value) {
        s["scanx"] =
            json!({"applied": self.applied, "hint_shown": self.shown, "hint_pending": self.pending, "hint_dismissed": self.dismissed});
    }
}

pub struct HintIn {
    /// the `.axelstra-wt` folder was just created for the first time
    pub new_root: bool,
    pub defender_active: bool,
    pub dev_drive: bool,
    pub excluded: bool,
    pub saved_shown: bool,
    pub saved_dismissed: bool,
}

/// The one-time notice: first creation of the folder, Defender really scanning, no Dev Drive, no exclusion yet, never
/// raised or dismissed before.
pub fn hint_due(i: &HintIn) -> bool {
    i.new_root && i.defender_active && !i.dev_drive && !i.excluded && !i.saved_shown && !i.saved_dismissed
}

/// The state the app shows. `q` is `None` when PowerShell was not asked (not Windows) or failed.
pub fn build_status(os: Os, fs: &dyn Fs, folders: &[Folder], q: Option<&Query>, saved: &Saved) -> Value {
    let state = match (os, q) {
        (Os::Windows, Some(q)) if q.active() => "active",
        (Os::Windows, Some(q)) if q.defender => "inactive",
        (Os::Windows, _) => "unavailable",
        _ => "n/a",
    };
    let readable = q.and_then(|q| q.exclusions.as_ref());
    let rows: Vec<Value> = folders
        .iter()
        .map(|f| {
            let path = Path::new(&f.path);
            let exists = fs.exists(path);
            let indexable = match os {
                Os::Windows => fs.attrs(path).ok().map(|a| a & NOT_CONTENT_INDEXED == 0),
                Os::MacOs => Some(!fs.exists(&path.join(MARKER))),
                Os::Other => None,
            };
            let recorded = saved.applied.iter().any(|r| canon(r) == canon(&f.path));
            let (excluded, source) = match (os, readable) {
                (Os::Windows, Some(list)) => (if list.iter().any(|e| covers(e, &f.path)) { "yes" } else { "no" }, Some("defender")),
                (Os::Windows, None) if recorded => ("yes", Some("recorded")),
                (Os::Windows, None) => ("unknown", None),
                _ => ("n/a", None),
            };
            let drive = drive_of(&f.path);
            let fs_name = drive.and_then(|d| q.and_then(|q| q.vols.get(&d))).cloned();
            json!({"project": f.project, "name": f.name, "path": f.path, "exists": exists, "indexable": indexable,
                "excluded": excluded, "source": source, "drive": drive.map(String::from), "fs": fs_name,
                "dev_drive": is_dev_drive(fs_name.as_deref())})
        })
        .collect();
    let can_apply = state == "active" && !folders.is_empty() && rows.iter().any(|r| r["excluded"] != "yes");
    let can_revert = os == Os::Windows && (!saved.applied.is_empty() || rows.iter().any(|r| r["excluded"] == "yes"));
    json!({
        "os": os.name(),
        "defender": {"state": state, "realtime": q.and_then(|q| q.realtime), "tamper": q.and_then(|q| q.tamper),
            "exclusions_readable": readable.is_some()},
        "folders": rows,
        "can_apply": can_apply,
        "can_revert": can_revert,
        "hint": saved.pending && !saved.dismissed,
        "hint_dismissed": saved.dismissed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// In-memory tree: path -> (attributes, is_dir, is_link).
    #[derive(Default)]
    struct FakeFs {
        nodes: Mutex<BTreeMap<PathBuf, (u32, bool, bool)>>,
        sets: Mutex<Vec<PathBuf>>,
    }

    impl FakeFs {
        fn dir(&self, p: &str, a: u32) {
            self.nodes.lock().unwrap().insert(PathBuf::from(p), (a, true, false));
        }
        fn file(&self, p: &str, a: u32) {
            self.nodes.lock().unwrap().insert(PathBuf::from(p), (a, false, false));
        }
        fn link(&self, p: &str) {
            self.nodes.lock().unwrap().insert(PathBuf::from(p), (0x400 | 0x10, true, true));
        }
        fn attr(&self, p: &str) -> u32 {
            self.nodes.lock().unwrap()[&PathBuf::from(p)].0
        }
    }

    impl Fs for FakeFs {
        fn attrs(&self, p: &Path) -> io::Result<u32> {
            self.nodes.lock().unwrap().get(p).map(|n| n.0).ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
        }
        fn set_attrs(&self, p: &Path, a: u32) -> io::Result<()> {
            self.sets.lock().unwrap().push(p.to_path_buf());
            self.nodes.lock().unwrap().get_mut(p).map(|n| n.0 = a).ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
        }
        fn is_dir(&self, p: &Path) -> bool {
            self.nodes.lock().unwrap().get(p).is_some_and(|n| n.1)
        }
        fn children(&self, p: &Path) -> io::Result<Vec<Entry>> {
            Ok(self
                .nodes
                .lock()
                .unwrap()
                .iter()
                .filter(|(k, _)| k.parent() == Some(p))
                .map(|(k, n)| Entry { path: k.clone(), is_dir: n.1, is_link: n.2 })
                .collect())
        }
        fn touch(&self, p: &Path) -> io::Result<()> {
            self.nodes.lock().unwrap().entry(p.to_path_buf()).or_insert((0x80, false, false));
            Ok(())
        }
        fn exists(&self, p: &Path) -> bool {
            self.nodes.lock().unwrap().contains_key(p)
        }
    }

    /// A Defender with a list of exclusions, behaving like the elevated script: idempotent, optionally ignoring changes.
    struct FakeRunner {
        list: RefCell<Vec<String>>,
        ignore: bool,
        error: Option<String>,
        calls: RefCell<Vec<(Op, Vec<String>)>>,
    }

    // the runner is used from one thread in these tests
    unsafe impl Sync for FakeRunner {}

    impl FakeRunner {
        fn new(list: &[&str]) -> FakeRunner {
            FakeRunner {
                list: RefCell::new(list.iter().map(|s| s.to_string()).collect()),
                ignore: false,
                error: None,
                calls: RefCell::new(vec![]),
            }
        }
    }

    impl Runner for FakeRunner {
        fn query(&self, _: &str) -> Result<String> {
            unreachable!()
        }
        fn elevate(&self, op: Op, paths: &[String]) -> Result<String> {
            self.calls.borrow_mut().push((op, paths.to_vec()));
            if let Some(e) = &self.error {
                return Ok(json!({"ok": false, "before": [], "after": [], "error": e}).to_string());
            }
            let before = self.list.borrow().clone();
            if !self.ignore {
                let mut l = self.list.borrow_mut();
                for p in paths {
                    let at = l.iter().position(|x| canon(x) == canon(p));
                    match (op, at) {
                        (Op::Add, None) => l.push(p.clone()),
                        (Op::Remove, Some(i)) => {
                            l.remove(i);
                        }
                        _ => {}
                    }
                }
            }
            let after = self.list.borrow().clone();
            Ok(json!({"ok": true, "before": before, "after": after, "error": null}).to_string())
        }
    }

    const WT: &str = r"C:\Users\x\Paradise Group\proj\.axelstra-wt";

    #[test]
    fn quoting_doubles_every_kind_of_single_quote_and_refuses_control_characters() {
        assert_eq!(ps_quote(WT).unwrap(), format!("'{WT}'"));
        assert_eq!(ps_quote("C:\\O'Brien\\.axelstra-wt").unwrap(), "'C:\\O''Brien\\.axelstra-wt'");
        assert_eq!(ps_quote("a\u{2019}b\u{2018}c").unwrap(), "'a\u{2019}\u{2019}b\u{2018}\u{2018}c'");
        // a path cannot close the literal and run something
        let evil = ps_quote("x'; Remove-Item C:\\ -Recurse; '").unwrap();
        assert_eq!(evil, "'x''; Remove-Item C:\\ -Recurse; '''");
        assert!(ps_quote("a\nb").is_err());
        assert!(ps_quote("a\0b").is_err());
    }

    #[test]
    fn the_elevated_script_carries_only_quoted_paths_and_guards_each_change() {
        let paths = vec![WT.to_string(), r"D:\a'b\.axelstra-wt".to_string()];
        let add = elevated_script(Op::Add, &paths, Path::new(r"C:\Temp\r esult.json")).unwrap();
        assert!(add.contains(&format!("@('{WT}','D:\\a''b\\.axelstra-wt')")));
        assert!(add.contains("Add-MpPreference -ExclusionPath $w") && !add.contains("Remove-MpPreference"));
        assert!(add.contains("if($idx -lt 0)"), "adds only what is missing");
        assert!(add.contains("'C:\\Temp\\r esult.json'"));
        let rm = elevated_script(Op::Remove, &paths, Path::new("r.json")).unwrap();
        assert!(rm.contains("Remove-MpPreference -ExclusionPath $before[$idx]") && !rm.contains("Add-MpPreference"));
        assert!(rm.contains("if($idx -ge 0)"), "removes only what is there");
        // the launcher hides the script in -EncodedCommand: no raw path on the command line
        let l = launcher_script(&add);
        assert!(l.contains("-Verb RunAs") && l.contains("-Wait") && l.contains("-EncodedCommand") && !l.contains("Paradise"));
        assert!(l.contains("1223"));
    }

    #[test]
    fn encoded_command_is_base64_of_utf16le() {
        assert_eq!(encode_command("a"), "YQA=");
        assert_eq!(encode_command("é"), "6QA=");
    }

    #[test]
    fn only_axelstra_wt_folders_are_accepted() {
        assert!(check_exclusion_path(WT).is_ok());
        assert!(check_exclusion_path(r"C:\Users\x\Projetos\.axelstra-wt\").is_ok());
        assert!(check_exclusion_path(r"\\srv\share\.axelstra-wt").is_ok());
        for bad in
            [r"C:\", r"C:\Users\x\Desktop\Projetos", r"C:\Users\x\.axelstra-wt-fake", "node.exe", r".axelstra-wt", r"relative\.axelstra-wt"]
        {
            assert!(check_exclusion_path(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn query_parsing_handles_a_string_an_array_null_and_the_no_admin_answer() {
        let one = parse_query(r#"{"defender":true,"av":true,"rtp":true,"tamper":false,"excl":"C:\\a\\.axelstra-wt","vols":{"C":"NTFS"}}"#);
        assert_eq!(one.exclusions, Some(vec![r"C:\a\.axelstra-wt".to_string()]));
        assert!(one.active() && one.vols[&'C'] == "NTFS" && one.tamper == Some(false));
        let many = parse_query(r#"{"defender":true,"av":true,"rtp":true,"excl":["a","b"],"vols":{}}"#);
        assert_eq!(many.exclusions, Some(vec!["a".into(), "b".into()]));
        // readable and empty is different from unreadable
        assert_eq!(parse_query(r#"{"defender":true,"excl":[]}"#).exclusions, Some(vec![]));
        assert_eq!(parse_query(r#"{"defender":true,"excl":null}"#).exclusions, None);
        assert_eq!(parse_query(r#"{"defender":true}"#).exclusions, None);
        let noadmin = parse_query(r#"{"defender":true,"av":true,"rtp":true,"excl":["N/A: Must be an administrator to view exclusions"]}"#);
        assert_eq!(noadmin.exclusions, None);
        // a BOM in front, third-party antivirus, garbage
        assert!(parse_query("\u{feff}{\"defender\":true,\"av\":true,\"rtp\":true}").active());
        assert!(!parse_query(r#"{"defender":true,"av":false,"rtp":false}"#).active());
        assert_eq!(parse_query("not json"), Query::default());
    }

    #[test]
    fn applying_twice_never_duplicates_and_revert_takes_back_only_those_paths() {
        let other = r"C:\keep\this";
        let r = FakeRunner::new(&[other]);
        let paths = vec![WT.to_string()];
        let first = run_change(&r, Op::Add, &paths).unwrap();
        assert_eq!(first, Change { changed: paths.clone(), unchanged: vec![] });
        let second = run_change(&r, Op::Add, &paths).unwrap();
        assert_eq!(second, Change { changed: vec![], unchanged: paths.clone() });
        assert_eq!(r.list.borrow().len(), 2, "one entry, not two");
        // another spelling of the same folder is the same entry
        let again = run_change(&r, Op::Add, &[format!("{}\\", WT.to_lowercase())]).unwrap();
        assert!(again.changed.is_empty());
        let back = run_change(&r, Op::Remove, &paths).unwrap();
        assert_eq!(back.changed, paths);
        assert_eq!(*r.list.borrow(), vec![other.to_string()], "the user's own exclusion stays");
        assert_eq!(run_change(&r, Op::Remove, &paths).unwrap().unchanged, paths);
    }

    fn code(e: &anyhow::Error) -> Option<String> {
        axelstra_core::msg::find(e).map(|c| c.code.clone())
    }

    #[test]
    fn a_change_defender_ignores_is_reported_as_blocked_not_as_success() {
        let mut r = FakeRunner::new(&[]);
        r.ignore = true;
        let e = run_change(&r, Op::Add, &[WT.to_string()]).unwrap_err();
        assert_eq!(code(&e).as_deref(), Some("scanx.defender_blocked"));
        let mut r = FakeRunner::new(&[WT]);
        r.ignore = true;
        let e = run_change(&r, Op::Remove, &[WT.to_string()]).unwrap_err();
        assert_eq!(code(&e).as_deref(), Some("scanx.defender_blocked"));
    }

    #[test]
    fn script_errors_are_classified_and_bad_paths_never_reach_the_runner() {
        let mut r = FakeRunner::new(&[]);
        r.error = Some("Operation failed with the following error: 0x800106ba. Tamper Protection".into());
        let e = run_change(&r, Op::Add, &[WT.to_string()]).unwrap_err();
        assert_eq!(code(&e).as_deref(), Some("scanx.defender_blocked"));
        r.error = Some("The RPC server is unavailable".into());
        let e = run_change(&r, Op::Add, &[WT.to_string()]).unwrap_err();
        assert_eq!(code(&e).as_deref(), Some("scanx.defender_failed"));
        let n = r.calls.borrow().len();
        for bad in [vec![r"C:\Users\x\Desktop\Projetos".to_string()], vec![], vec!["C:\\".to_string()]] {
            assert!(run_change(&r, Op::Add, &bad).is_err());
        }
        assert_eq!(r.calls.borrow().len(), n, "nothing was elevated");
        let many: Vec<String> = (0..=MAX_PATHS).map(|i| format!("C:\\p{i}\\.axelstra-wt")).collect();
        assert!(run_change(&r, Op::Add, &many).is_err());
    }

    #[test]
    fn the_recorded_list_follows_add_and_remove() {
        let a = vec!["C:\\a\\.axelstra-wt".to_string()];
        let b = vec!["C:\\b\\.axelstra-wt".to_string()];
        let both = recorded_after(&a, Op::Add, &b);
        assert_eq!(both.len(), 2);
        assert_eq!(recorded_after(&both, Op::Add, &b).len(), 2, "no duplicate");
        assert_eq!(recorded_after(&both, Op::Remove, &["c:\\A\\.axelstra-wt\\".to_string()]), b);
    }

    #[test]
    fn procs_parse_one_object_or_a_list() {
        let one = parse_procs(r#"{"Name":"MsMpEng","IDProcess":4,"PercentProcessorTime":64,"WorkingSet":100}"#);
        assert_eq!(one.len(), 1);
        assert_eq!(one[0]["cpu"], 64);
        let two = parse_procs(
            r#"[{"Name":"a","IDProcess":1,"PercentProcessorTime":0,"WorkingSet":1},{"Name":"b","IDProcess":2,"PercentProcessorTime":1,"WorkingSet":2}]"#,
        );
        assert_eq!(two.len(), 2);
        assert!(parse_procs("").is_empty() && parse_procs("null").is_empty());
    }

    fn folder(path: &str) -> Folder {
        Folder { project: "p1".into(), name: "proj".into(), path: path.into() }
    }

    fn active(excl: Option<Vec<String>>, vols: &[(char, &str)]) -> Query {
        Query {
            defender: true,
            antivirus: Some(true),
            realtime: Some(true),
            tamper: Some(true),
            exclusions: excl,
            vols: vols.iter().map(|(c, f)| (*c, f.to_string())).collect(),
        }
    }

    #[test]
    fn status_is_not_applicable_off_windows_and_unknown_without_admin() {
        let fs = FakeFs::default();
        let f = [folder(WT)];
        let s = build_status(Os::Other, &fs, &f, None, &Saved::default());
        assert_eq!(s["defender"]["state"], "n/a");
        assert_eq!(s["folders"][0]["excluded"], "n/a");
        assert_eq!(s["can_apply"], false);
        // Windows, Defender active, exclusions unreadable, nothing recorded
        fs.dir(WT, 0x10);
        let q = active(None, &[('C', "NTFS")]);
        let s = build_status(Os::Windows, &fs, &f, Some(&q), &Saved::default());
        assert_eq!(s["defender"]["state"], "active");
        assert_eq!(s["defender"]["exclusions_readable"], false);
        assert_eq!(s["folders"][0]["excluded"], "unknown");
        assert_eq!(s["folders"][0]["indexable"], true);
        assert_eq!(s["folders"][0]["fs"], "NTFS");
        assert_eq!(s["can_apply"], true);
        assert_eq!(s["can_revert"], false);
        // recorded as applied
        let saved = Saved { applied: vec![format!("{}\\", WT.to_lowercase())], ..Default::default() };
        let s = build_status(Os::Windows, &fs, &f, Some(&q), &saved);
        assert_eq!((s["folders"][0]["excluded"].as_str(), s["folders"][0]["source"].as_str()), (Some("yes"), Some("recorded")));
        assert_eq!(s["can_apply"], false);
        assert_eq!(s["can_revert"], true);
    }

    #[test]
    fn status_reads_exclusions_when_allowed_and_a_parent_exclusion_covers_the_folder() {
        let fs = FakeFs::default();
        let f = [folder(WT)];
        let q = active(Some(vec![r"C:\USERS\X\PARADISE GROUP".into()]), &[('C', "ReFS")]);
        let s = build_status(Os::Windows, &fs, &f, Some(&q), &Saved::default());
        assert_eq!(s["folders"][0]["excluded"], "yes");
        assert_eq!(s["folders"][0]["source"], "defender");
        assert_eq!(s["folders"][0]["dev_drive"], true);
        let q = active(Some(vec![r"C:\Users\x\Paradise".into()]), &[]);
        let s = build_status(Os::Windows, &fs, &f, Some(&q), &Saved::default());
        assert_eq!(s["folders"][0]["excluded"], "no", "a sibling prefix is not a parent");
        // Defender off (another antivirus)
        let off = Query { defender: true, antivirus: Some(false), realtime: Some(false), ..Default::default() };
        assert_eq!(build_status(Os::Windows, &fs, &f, Some(&off), &Saved::default())["defender"]["state"], "inactive");
        assert_eq!(build_status(Os::Windows, &fs, &f, None, &Saved::default())["defender"]["state"], "unavailable");
    }

    #[test]
    fn the_notice_is_one_shot_and_stays_away_on_dev_drives_and_when_excluded() {
        let base =
            HintIn { new_root: true, defender_active: true, dev_drive: false, excluded: false, saved_shown: false, saved_dismissed: false };
        assert!(hint_due(&base));
        assert!(!hint_due(&HintIn { new_root: false, ..base }));
        assert!(!hint_due(&HintIn { defender_active: false, ..base }));
        assert!(!hint_due(&HintIn { dev_drive: true, ..base }));
        assert!(!hint_due(&HintIn { excluded: true, ..base }));
        assert!(!hint_due(&HintIn { saved_shown: true, ..base }));
        assert!(!hint_due(&HintIn { saved_dismissed: true, ..base }));
        // pending until dismissed
        let pending = Saved { pending: true, shown: true, ..Default::default() };
        let fs = FakeFs::default();
        assert_eq!(build_status(Os::Windows, &fs, &[], None, &pending)["hint"], true);
        let gone = Saved { dismissed: true, ..pending };
        assert_eq!(build_status(Os::Windows, &fs, &[], None, &gone)["hint"], false);
    }

    #[test]
    fn saved_state_round_trips_through_settings() {
        let mut s = json!({"terse": "off"});
        Saved { applied: vec!["a".into()], shown: true, pending: true, dismissed: false }.write_into(&mut s);
        assert_eq!(s["terse"], "off");
        assert_eq!(Saved::from_settings(&s), Saved { applied: vec!["a".into()], shown: true, pending: true, dismissed: false });
        assert_eq!(Saved::from_settings(&json!({})), Saved::default());
    }

    #[test]
    fn the_flag_keeps_other_attributes_and_drops_normal() {
        assert_eq!(with_flag(0x20), 0x2020);
        assert_eq!(with_flag(0x01 | 0x02), 0x2003);
        assert_eq!(with_flag(0x80), 0x2000, "NORMAL is only valid alone");
        assert_eq!(with_flag(0x2010), 0x2010);
    }

    #[test]
    fn windows_flags_the_root_and_existing_trees_but_not_links_and_is_idempotent() {
        let fs = FakeFs::default();
        let root = r"C:\wt\.axelstra-wt";
        fs.dir(root, 0x10);
        fs.dir(&format!("{root}\\old"), 0x10);
        fs.dir(&format!("{root}\\old\\node_modules"), 0x10);
        fs.file(&format!("{root}\\old\\node_modules\\a.js"), 0x21);
        fs.link(&format!("{root}\\old\\junction"));
        fs.dir(&format!("{root}\\fresh"), 0x10 | NOT_CONTENT_INDEXED);
        fs.file(&format!("{root}\\fresh\\unvisited.txt"), 0x20);
        let r = sweep_root(&fs, Os::Windows, Path::new(root), 100, 100);
        assert_eq!(r.failed, 0);
        assert!(!r.capped);
        for p in [root, r"C:\wt\.axelstra-wt\old", r"C:\wt\.axelstra-wt\old\node_modules"] {
            assert_ne!(fs.attr(p) & NOT_CONTENT_INDEXED, 0, "{p}");
        }
        assert_eq!(fs.attr(r"C:\wt\.axelstra-wt\old\node_modules\a.js"), 0x2021, "other bits kept");
        assert_eq!(fs.attr(r"C:\wt\.axelstra-wt\old\junction") & NOT_CONTENT_INDEXED, 0, "a link is left alone");
        assert_eq!(fs.attr(r"C:\wt\.axelstra-wt\fresh\unvisited.txt"), 0x20, "a flagged worktree counts as done");
        let sets = fs.sets.lock().unwrap().len();
        let again = sweep_root(&fs, Os::Windows, Path::new(root), 100, 100);
        assert_eq!(again.changed, 0);
        assert_eq!(fs.sets.lock().unwrap().len(), sets, "second run writes nothing");
    }

    #[test]
    fn a_capped_walk_leaves_the_worktree_unflagged_so_the_next_start_continues() {
        let fs = FakeFs::default();
        let root = r"C:\wt\.axelstra-wt";
        fs.dir(root, 0x10);
        fs.dir(&format!("{root}\\big"), 0x10);
        for i in 0..10 {
            fs.file(&format!("{root}\\big\\f{i}"), 0x20);
        }
        let r = sweep_root(&fs, Os::Windows, Path::new(root), 4, 1000);
        assert!(r.capped);
        assert_eq!(fs.attr(r"C:\wt\.axelstra-wt\big") & NOT_CONTENT_INDEXED, 0);
        let r = sweep_root(&fs, Os::Windows, Path::new(root), 100, 1000);
        assert!(!r.capped);
        assert_ne!(fs.attr(r"C:\wt\.axelstra-wt\big") & NOT_CONTENT_INDEXED, 0);
        let again = sweep_root(&fs, Os::Windows, Path::new(root), 100, 1000);
        assert_eq!(again.changed, 0);
    }

    #[test]
    fn macos_gets_the_marker_once_and_linux_nothing() {
        let fs = FakeFs::default();
        let root = Path::new("/p/.axelstra-wt");
        fs.dir("/p/.axelstra-wt", 0);
        assert_eq!(protect_root(&fs, Os::MacOs, root).changed, 1);
        assert!(fs.exists(&root.join(MARKER)));
        let again = protect_root(&fs, Os::MacOs, root);
        assert_eq!((again.changed, again.already), (0, 1));
        let linux = FakeFs::default();
        linux.dir("/p/.axelstra-wt", 0);
        assert_eq!(sweep_root(&linux, Os::Other, root, 10, 10), Applied::default());
        assert!(linux.sets.lock().unwrap().is_empty() && !linux.exists(&root.join(MARKER)));
    }

    #[test]
    fn folders_come_from_the_parent_of_each_project_and_are_deduplicated() {
        let ps = vec![
            ("a".to_string(), "A".to_string(), r"C:\Users\x\Paradise Group\proj".to_string()),
            ("b".to_string(), "B".to_string(), r"C:\Users\x\Paradise Group\other".to_string()),
            ("c".to_string(), "C".to_string(), r"D:\code\c".to_string()),
        ];
        let fs = folders_of(&ps);
        if cfg!(windows) {
            assert_eq!(fs[0].path, r"C:\Users\x\Paradise Group\.axelstra-wt");
            assert_eq!(unique_paths(&fs).len(), 2);
            assert_eq!(drives_of(&fs), vec!['C', 'D']);
        }
        assert_eq!(fs.len(), 3);
    }

    #[test]
    fn the_real_filesystem_wrapper_works_in_a_tempdir_only() {
        let d = std::env::temp_dir().join(format!("axl-scanx-{}", std::process::id()));
        let root = d.join(WT_DIR);
        let wt = root.join("w1");
        std::fs::create_dir_all(wt.join("node_modules")).unwrap();
        std::fs::write(wt.join("node_modules").join("a.js"), "x").unwrap();
        let os = Os::current();
        let r = sweep_root(&RealFs, os, &root, 1000, 1000);
        assert_eq!(r.failed, 0);
        #[cfg(windows)]
        {
            for p in [&root, &wt, &wt.join("node_modules"), &wt.join("node_modules").join("a.js")] {
                assert_ne!(RealFs.attrs(p).unwrap() & NOT_CONTENT_INDEXED, 0, "{}", p.display());
            }
            // born flagged below a flagged folder
            std::fs::write(wt.join("later.txt"), "x").unwrap();
            assert_ne!(RealFs.attrs(&wt.join("later.txt")).unwrap() & NOT_CONTENT_INDEXED, 0);
            assert_eq!(sweep_root(&RealFs, os, &root, 1000, 1000).changed, 0);
        }
        #[cfg(target_os = "macos")]
        assert!(root.join(MARKER).exists());
        let _ = std::fs::remove_dir_all(&d);
    }
}
