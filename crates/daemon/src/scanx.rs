//! Keeps the Windows and macOS file indexers off the worktree folders (`<repo parent>/.axelstra-wt`): the node_modules and
//! target trees of a score of worktrees made Windows Search (SearchIndexer) and Spotlight eat the CPU.
//!
//! No admin, silent, idempotent, reversible by hand. Windows gets `FILE_ATTRIBUTE_NOT_CONTENT_INDEXED` on the
//! `.axelstra-wt` folder and its worktrees (NTFS gives it to everything created below a flagged folder, so only existing
//! content needs a walk, in a low-priority thread with a cap); macOS gets an empty `.metadata_never_index`; Linux gets
//! nothing. Triggered by events: a worktree or run creating the folder and the daemon starting. No polling.
//! Everything goes through the `Fs` trait so tests never touch the machine.

use std::io;
use std::path::{Path, PathBuf};

/// Name of the folder that holds the worktrees.
pub const WT_DIR: &str = ".axelstra-wt";
/// macOS: Spotlight skips a folder that holds this file.
pub const MARKER: &str = ".metadata_never_index";
pub const NOT_CONTENT_INDEXED: u32 = 0x2000;
const ATTR_NORMAL: u32 = 0x80;
#[cfg(windows)]
const ATTR_REPARSE: u32 = 0x400;
/// Attribute changes made for one tree before the walk stops (it goes on at the next start).
pub const MAX_SET: usize = 50_000;
/// Entries looked at for one tree before the walk stops.
pub const MAX_VISIT: usize = 1_000_000;

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

/// Sweeps the roots in a low-priority thread, one after the other. By event (daemon start); takes no lock.
pub fn spawn_sweep(roots: Vec<PathBuf>) {
    let os = Os::current();
    let roots: Vec<PathBuf> = roots.into_iter().filter(|r| r.is_dir()).collect();
    if os == Os::Other || roots.is_empty() {
        return;
    }
    let _ = std::thread::Builder::new().name("scanx-sweep".into()).spawn(move || {
        background_thread();
        for r in roots {
            let res = sweep_root(&RealFs, os, &r, MAX_SET, MAX_VISIT);
            if res.failed > 0 || res.capped {
                eprintln!("scanx: {} changed, {} failed{}", res.changed, res.failed, if res.capped { ", capped" } else { "" });
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::Mutex;

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
        // forward slashes: `Path::parent` only splits on them on every platform
        let root = "/wt/.axelstra-wt";
        fs.dir(root, 0x10);
        fs.dir(&format!("{root}/old"), 0x10);
        fs.dir(&format!("{root}/old/node_modules"), 0x10);
        fs.file(&format!("{root}/old/node_modules/a.js"), 0x21);
        fs.link(&format!("{root}/old/junction"));
        fs.dir(&format!("{root}/fresh"), 0x10 | NOT_CONTENT_INDEXED);
        fs.file(&format!("{root}/fresh/unvisited.txt"), 0x20);
        let r = sweep_root(&fs, Os::Windows, Path::new(root), 100, 100);
        assert_eq!(r.failed, 0);
        assert!(!r.capped);
        for p in [root, "/wt/.axelstra-wt/old", "/wt/.axelstra-wt/old/node_modules"] {
            assert_ne!(fs.attr(p) & NOT_CONTENT_INDEXED, 0, "{p}");
        }
        assert_eq!(fs.attr("/wt/.axelstra-wt/old/node_modules/a.js"), 0x2021, "other bits kept");
        assert_eq!(fs.attr("/wt/.axelstra-wt/old/junction") & NOT_CONTENT_INDEXED, 0, "a link is left alone");
        assert_eq!(fs.attr("/wt/.axelstra-wt/fresh/unvisited.txt"), 0x20, "a flagged worktree counts as done");
        let sets = fs.sets.lock().unwrap().len();
        let again = sweep_root(&fs, Os::Windows, Path::new(root), 100, 100);
        assert_eq!(again.changed, 0);
        assert_eq!(fs.sets.lock().unwrap().len(), sets, "second run writes nothing");
    }

    #[test]
    fn a_capped_walk_leaves_the_worktree_unflagged_so_the_next_start_continues() {
        let fs = FakeFs::default();
        let root = "/wt/.axelstra-wt";
        fs.dir(root, 0x10);
        fs.dir(&format!("{root}/big"), 0x10);
        for i in 0..10 {
            fs.file(&format!("{root}/big/f{i}"), 0x20);
        }
        let r = sweep_root(&fs, Os::Windows, Path::new(root), 4, 1000);
        assert!(r.capped);
        assert_eq!(fs.attr("/wt/.axelstra-wt/big") & NOT_CONTENT_INDEXED, 0);
        let r = sweep_root(&fs, Os::Windows, Path::new(root), 100, 1000);
        assert!(!r.capped);
        assert_ne!(fs.attr("/wt/.axelstra-wt/big") & NOT_CONTENT_INDEXED, 0);
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
