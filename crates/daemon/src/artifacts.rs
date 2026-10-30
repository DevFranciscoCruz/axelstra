//! Regenerable build artifacts of a worktree: cargo's `target/`, `node_modules/` and the caches of JS tools. They grow
//! to gigabytes per worktree and come back with the next build, so the janitor takes them from worktrees nobody uses
//! (see `server/disk.rs`). Never the worktree, never a tracked file, never through a symlink or junction.
//!
//! Pure: the file system and git are behind `Tree` (tests inject one), the rules are plain functions of `Facts`.

use std::path::{Path, PathBuf};

/// The caches of JS tools, taken only when git says the folder is ignored.
pub const CACHES: &[&str] = &[".next", ".turbo", ".parcel-cache", ".svelte-kit", ".vite", "dist", "build", "out"];

/// Default and bounds of `janitor.artifact_quiet_hours`.
pub const QUIET_HOURS: u64 = 6;
pub const QUIET_HOURS_MAX: u64 = 720;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Target,
    NodeModules,
    Cache,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Target => "target",
            Kind::NodeModules => "node_modules",
            Kind::Cache => "cache",
        }
    }
}

/// One reclaimable folder of a worktree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dir {
    pub path: PathBuf,
    /// relative to the worktree, with `/`
    pub rel: String,
    pub kind: Kind,
}

/// What the artifacts code needs from the file system and git.
pub trait Tree {
    fn is_file(&self, p: &Path) -> bool;
    /// a directory that is not itself a symlink or junction
    fn is_plain_dir(&self, p: &Path) -> bool;
    /// a symlink, junction or any other reparse point
    fn is_link(&self, p: &Path) -> bool;
    /// the plain (not linked) directories right below `p`
    fn child_dirs(&self, p: &Path) -> Vec<PathBuf>;
    /// `git check-ignore` says `rel` is ignored in the worktree `wt`
    fn ignored(&self, wt: &Path, rel: &str) -> bool;
    /// `git ls-files -- rel` lists something: a tracked file lives inside
    fn tracked(&self, wt: &Path, rel: &str) -> bool;
    /// bytes of the files below `p`, never through a link
    fn size(&self, p: &Path) -> u64;
    /// same, leaving out the folders `skip` (measured apart) and `.git`
    fn size_except(&self, p: &Path, skip: &[PathBuf]) -> u64;
    /// removes `p` without following any link inside it
    fn remove(&self, p: &Path) -> std::io::Result<()>;
    fn mtime_ms(&self, p: &Path) -> Option<u64>;
}

/// Where reclaimable folders may be, without asking git: `target/` next to the root `Cargo.toml`, `node_modules/` next to
/// a `package.json` in the root or a folder right below it, and the `CACHES` of those same folders. Only plain folders
/// that exist (a linked one is skipped). Cheap: the automatic pass looks at this before it runs any git.
pub fn positions(t: &dyn Tree, wt: &Path) -> Vec<Dir> {
    let mut out = vec![];
    let mut consider = |path: PathBuf, rel: String, kind: Kind| {
        if !t.is_link(&path) && t.is_plain_dir(&path) {
            out.push(Dir { path, rel, kind });
        }
    };
    if t.is_file(&wt.join("Cargo.toml")) {
        consider(wt.join("target"), "target".into(), Kind::Target);
    }
    let mut bases: Vec<(PathBuf, String)> = vec![];
    if t.is_file(&wt.join("package.json")) {
        bases.push((wt.to_path_buf(), String::new()));
    }
    let mut kids = t.child_dirs(wt);
    kids.sort();
    for c in kids {
        let Some(name) = c.file_name().map(|n| n.to_string_lossy().into_owned()) else { continue };
        if name == ".git" || name == "node_modules" || name == "target" || CACHES.contains(&name.as_str()) || t.is_link(&c) {
            continue;
        }
        if t.is_file(&c.join("package.json")) {
            bases.push((c, name));
        }
    }
    let rel_of = |base: &str, leaf: &str| if base.is_empty() { leaf.to_string() } else { format!("{base}/{leaf}") };
    for (b, rel) in &bases {
        consider(b.join("node_modules"), rel_of(rel, "node_modules"), Kind::NodeModules);
        for c in CACHES {
            consider(b.join(c), rel_of(rel, c), Kind::Cache);
        }
    }
    out
}

/// The reclaimable folders of the worktree `wt`: the `positions` that git says are ignored and hold no tracked file.
pub fn candidates(t: &dyn Tree, wt: &Path) -> Vec<Dir> {
    positions(t, wt).into_iter().filter(|d| t.ignored(wt, &d.rel) && !t.tracked(wt, &d.rel)).collect()
}

/// The candidates with their sizes in bytes.
pub fn measure(t: &dyn Tree, wt: &Path) -> Vec<(Dir, u64)> {
    candidates(t, wt)
        .into_iter()
        .map(|d| {
            let n = t.size(&d.path);
            (d, n)
        })
        .collect()
}

/// The candidates with their sizes, and the bytes of the whole worktree (its files plus the candidates), walking each
/// file once.
pub fn measure_all(t: &dyn Tree, wt: &Path) -> (Vec<(Dir, u64)>, u64) {
    let dirs = measure(t, wt);
    let skip: Vec<PathBuf> = dirs.iter().map(|(d, _)| d.path.clone()).collect();
    let total = t.size_except(wt, &skip) + dirs.iter().map(|(_, n)| n).sum::<u64>();
    (dirs, total)
}

/// Bytes to MB, rounded down.
pub fn mb(bytes: u64) -> u64 {
    bytes / (1024 * 1024)
}

/// What a reclaim did: the folders that went (with their size) and the first failure.
#[derive(Debug, Default)]
pub struct Reclaimed {
    pub removed: Vec<(Dir, u64)>,
    pub freed: u64,
    pub failed: Option<(PathBuf, String)>,
}

/// Removes the folders (already measured). One that fails stops nothing: the others still go.
pub fn reclaim(t: &dyn Tree, dirs: &[(Dir, u64)]) -> Reclaimed {
    let mut r = Reclaimed::default();
    for (d, n) in dirs {
        // checked again: it may have become a link since it was listed
        if t.is_link(&d.path) {
            continue;
        }
        match t.remove(&d.path) {
            Ok(()) => {
                r.freed += n;
                r.removed.push((d.clone(), *n));
            }
            Err(e) => {
                if r.failed.is_none() {
                    r.failed = Some((d.path.clone(), e.to_string()));
                }
            }
        }
    }
    r
}

/// The quiet spell of `janitor.artifact_quiet_hours` (1..720, default 6), in ms.
pub fn quiet_ms(settings: &serde_json::Value) -> u64 {
    let h = settings["janitor"]["artifact_quiet_hours"].as_u64().filter(|h| (1..=QUIET_HOURS_MAX).contains(h)).unwrap_or(QUIET_HOURS);
    h * 3600 * 1000
}

/// Validates a `janitor` settings patch: `{artifact_quiet_hours}`. The keys present change, the rest stays.
pub fn validate(saved: &serde_json::Value, patch: &serde_json::Value) -> Option<serde_json::Value> {
    let mut out = saved["janitor"].as_object().cloned().unwrap_or_default();
    for (k, v) in patch.as_object()? {
        match k.as_str() {
            "artifact_quiet_hours" => {
                let h = v.as_u64().filter(|h| (1..=QUIET_HOURS_MAX).contains(h))?;
                out.insert(k.clone(), h.into());
            }
            _ => return None,
        }
    }
    Some(serde_json::Value::Object(out))
}

/// The last moment the worktree was used, as its files tell: its git `HEAD`, `index` and `logs/HEAD` (a checkout, a
/// commit, a `git add`). The `.git` of a linked worktree is a file naming its git dir.
pub fn files_used_ms(t: &dyn Tree, wt: &Path) -> Option<u64> {
    let dot = wt.join(".git");
    let gitdir = if t.is_file(&dot) {
        std::fs::read_to_string(&dot)
            .ok()
            .and_then(|s| s.lines().find_map(|l| l.strip_prefix("gitdir:").map(|g| g.trim().to_string())))
            .map(|g| {
                let g = PathBuf::from(g);
                if g.is_absolute() {
                    g
                } else {
                    wt.join(g)
                }
            })
            .unwrap_or(dot)
    } else {
        dot
    };
    ["HEAD", "index", "logs/HEAD"].iter().filter_map(|f| t.mtime_ms(&gitdir.join(f))).max()
}

/// What decides whether a worktree's artifacts may go now.
#[derive(Clone, Copy, Debug, Default)]
pub struct Facts {
    /// the project's own checkout
    pub main: bool,
    /// a live or hibernated session, a flow, a run or a build (`axelstra lock`, `mem.hold`) uses it
    pub busy: bool,
    /// last use (a session leaving it, its git files); `None`: nothing tells, counts as long ago
    pub last_used_ms: Option<u64>,
    pub now_ms: u64,
    pub quiet_ms: u64,
    /// the user asked: only the quiet rule is skipped
    pub force: bool,
}

/// Why the artifacts of `path` must stay, or `Ok` when they may go.
pub fn check(path: &str, f: &Facts) -> anyhow::Result<()> {
    if f.main {
        return Err(axelstra_core::coded!("artifacts.main", "{path} is the project's main checkout: its builds are kept", path = path));
    }
    if f.busy {
        return Err(axelstra_core::coded!("artifacts.busy", "{path} is in use (a session or a build): its builds are kept", path = path));
    }
    if !f.force && due_in(f).is_some() {
        let hours = f.quiet_ms / 3_600_000;
        return Err(axelstra_core::coded!(
            "artifacts.recent",
            "{path} was used in the last {hours} h: its builds are kept",
            path = path,
            hours = hours
        ));
    }
    Ok(())
}

/// How long until the worktree has been quiet for `quiet_ms` (`None`: it already is, or nothing tells when it was used).
pub fn due_in(f: &Facts) -> Option<u64> {
    let at = f.last_used_ms?.saturating_add(f.quiet_ms);
    (at > f.now_ms).then(|| at - f.now_ms)
}

/// The real file system and git.
pub struct OsTree;

#[cfg(windows)]
fn md_is_link(md: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    // FILE_ATTRIBUTE_REPARSE_POINT: symlink, junction, mount point
    md.file_attributes() & 0x400 != 0 || md.file_type().is_symlink()
}

#[cfg(not(windows))]
fn md_is_link(md: &std::fs::Metadata) -> bool {
    md.file_type().is_symlink()
}

/// Removes a link itself, never what it points to (a link to a folder is a directory entry on Windows).
fn unlink(p: &Path, md: &std::fs::Metadata) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if md.file_attributes() & 0x10 != 0 {
            return std::fs::remove_dir(p);
        }
    }
    let _ = md;
    std::fs::remove_file(p)
}

/// Walks `root` without entering links: `f(path, metadata)` for every entry below it. `enter(dir)` false keeps the
/// walk out of a folder.
fn walk(root: &Path, enter: &dyn Fn(&Path) -> bool, f: &mut dyn FnMut(&Path, &std::fs::Metadata)) {
    let mut stack = vec![axelstra_core::git::verbatim(root)];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            let Ok(md) = std::fs::symlink_metadata(&p) else { continue };
            f(&p, &md);
            if md.is_dir() && !md_is_link(&md) && enter(&p) {
                stack.push(p);
            }
        }
    }
}

impl Tree for OsTree {
    fn is_file(&self, p: &Path) -> bool {
        std::fs::symlink_metadata(p).is_ok_and(|m| m.is_file())
    }

    fn is_plain_dir(&self, p: &Path) -> bool {
        std::fs::symlink_metadata(p).is_ok_and(|m| m.is_dir() && !md_is_link(&m))
    }

    fn is_link(&self, p: &Path) -> bool {
        std::fs::symlink_metadata(p).is_ok_and(|m| md_is_link(&m))
    }

    fn child_dirs(&self, p: &Path) -> Vec<PathBuf> {
        let Ok(rd) = std::fs::read_dir(p) else { return vec![] };
        rd.flatten().map(|e| e.path()).filter(|c| self.is_plain_dir(c)).collect()
    }

    fn ignored(&self, wt: &Path, rel: &str) -> bool {
        // the trailing slash lets a directory-only pattern (`target/`) match
        axelstra_core::git::run(wt, &["check-ignore", "-q", "--", &format!("{rel}/")]).is_ok_and(|o| o.ok)
    }

    fn tracked(&self, wt: &Path, rel: &str) -> bool {
        // when git cannot answer, assume the worst
        axelstra_core::git::run(wt, &["ls-files", "--cached", "--", rel]).map_or(true, |o| !o.ok || !o.stdout.trim().is_empty())
    }

    fn size(&self, p: &Path) -> u64 {
        self.size_except(p, &[])
    }

    fn size_except(&self, p: &Path, skip: &[PathBuf]) -> u64 {
        let skip: Vec<PathBuf> = skip.iter().map(|s| axelstra_core::git::verbatim(s)).collect();
        let enter = |d: &Path| !skip.iter().any(|s| s == d) && d.file_name().is_none_or(|n| n != ".git");
        let mut n = 0;
        walk(p, &enter, &mut |_, md| {
            if md.is_file() {
                n += md.len();
            }
        });
        n
    }

    fn remove(&self, p: &Path) -> std::io::Result<()> {
        let md = std::fs::symlink_metadata(p)?;
        if md_is_link(&md) {
            return Err(std::io::Error::other("is a link"));
        }
        // every link inside goes first, as a link: the removal below then has nothing to follow out of the folder
        let mut links = vec![];
        walk(p, &|_| true, &mut |q, md| {
            if md_is_link(md) {
                links.push((q.to_path_buf(), md.clone()));
            }
        });
        for (q, md) in links {
            unlink(&q, &md)?;
        }
        axelstra_core::git::remove_tree(p)
    }

    fn mtime_ms(&self, p: &Path) -> Option<u64> {
        let t = std::fs::metadata(p).ok()?.modified().ok()?;
        t.duration_since(std::time::UNIX_EPOCH).ok().map(|d| d.as_millis() as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, HashSet};
    use std::sync::Mutex;

    /// An in-memory tree: files, plain dirs, links, ignored and tracked paths, sizes.
    #[derive(Default)]
    struct Fake {
        files: HashSet<PathBuf>,
        dirs: HashSet<PathBuf>,
        links: HashSet<PathBuf>,
        ignored: HashSet<String>,
        tracked: HashSet<String>,
        sizes: HashMap<PathBuf, u64>,
        removed: Mutex<Vec<PathBuf>>,
        fail: HashSet<PathBuf>,
    }

    impl Tree for Fake {
        fn is_file(&self, p: &Path) -> bool {
            self.files.contains(p)
        }
        fn is_plain_dir(&self, p: &Path) -> bool {
            self.dirs.contains(p)
        }
        fn is_link(&self, p: &Path) -> bool {
            self.links.contains(p)
        }
        fn child_dirs(&self, p: &Path) -> Vec<PathBuf> {
            self.dirs.iter().filter(|d| d.parent() == Some(p)).cloned().collect()
        }
        fn ignored(&self, _: &Path, rel: &str) -> bool {
            self.ignored.contains(rel)
        }
        fn tracked(&self, _: &Path, rel: &str) -> bool {
            self.tracked.contains(rel)
        }
        fn size(&self, p: &Path) -> u64 {
            self.sizes.get(p).copied().unwrap_or(0)
        }
        fn size_except(&self, p: &Path, _: &[PathBuf]) -> u64 {
            self.size(p)
        }
        fn remove(&self, p: &Path) -> std::io::Result<()> {
            if self.fail.contains(p) {
                return Err(std::io::Error::other("held"));
            }
            self.removed.lock().unwrap().push(p.to_path_buf());
            Ok(())
        }
        fn mtime_ms(&self, _: &Path) -> Option<u64> {
            None
        }
    }

    fn wt() -> PathBuf {
        PathBuf::from("/w")
    }

    #[test]
    fn target_needs_cargo_toml_and_node_modules_needs_package_json() {
        let mut f = Fake::default();
        let w = wt();
        f.dirs.extend([w.join("target"), w.join("node_modules"), w.join("app"), w.join("app/node_modules"), w.join("docs")]);
        f.dirs.insert(w.join("docs/node_modules"));
        f.ignored.extend(["target", "node_modules", "app/node_modules", "docs/node_modules"].map(String::from));
        // no Cargo.toml, no package.json anywhere: nothing
        assert!(candidates(&f, &w).is_empty());
        f.files.insert(w.join("Cargo.toml"));
        f.files.insert(w.join("app/package.json"));
        let c = candidates(&f, &w);
        let rels: Vec<_> = c.iter().map(|d| (d.rel.as_str(), d.kind)).collect();
        assert_eq!(rels, vec![("target", Kind::Target), ("app/node_modules", Kind::NodeModules)]);
    }

    #[test]
    fn caches_only_when_ignored_and_nothing_tracked_or_linked() {
        let mut f = Fake::default();
        let w = wt();
        f.files.insert(w.join("package.json"));
        f.dirs.extend([w.join("dist"), w.join(".next"), w.join("build"), w.join("node_modules")]);
        f.links.insert(w.join("node_modules"));
        f.ignored.extend(["dist", ".next", "node_modules"].map(String::from));
        // build is not ignored, .next holds a tracked file, node_modules is a link
        f.tracked.insert(".next".into());
        let c = candidates(&f, &w);
        assert_eq!(c.iter().map(|d| d.rel.as_str()).collect::<Vec<_>>(), vec!["dist"]);
        assert_eq!(c[0].kind, Kind::Cache);
    }

    #[test]
    fn a_linked_child_folder_is_not_entered() {
        let mut f = Fake::default();
        let w = wt();
        f.dirs.extend([w.join("ext"), w.join("ext/node_modules")]);
        f.links.insert(w.join("ext"));
        f.files.insert(w.join("ext/package.json"));
        f.ignored.insert("ext/node_modules".into());
        assert!(candidates(&f, &w).is_empty());
    }

    #[test]
    fn reclaim_counts_what_went_and_keeps_going_after_a_failure() {
        let mut f = Fake::default();
        let w = wt();
        let a = Dir { path: w.join("target"), rel: "target".into(), kind: Kind::Target };
        let b = Dir { path: w.join("node_modules"), rel: "node_modules".into(), kind: Kind::NodeModules };
        f.fail.insert(a.path.clone());
        let r = reclaim(&f, &[(a.clone(), 10), (b.clone(), 5)]);
        assert_eq!(r.freed, 5);
        assert_eq!(r.removed, vec![(b, 5)]);
        assert_eq!(r.failed.unwrap().0, a.path);
    }

    #[test]
    fn rules_main_busy_recent_and_force() {
        let h = 3_600_000;
        let base = Facts { main: false, busy: false, last_used_ms: Some(10 * h), now_ms: 12 * h, quiet_ms: 6 * h, force: false };
        let code = |f: &Facts| check("/w", f).err().and_then(|e| axelstra_core::msg::find(&e).map(|c| c.code.clone()));
        assert_eq!(code(&base).as_deref(), Some("artifacts.recent"));
        assert_eq!(due_in(&base), Some(4 * h));
        assert!(check("/w", &Facts { force: true, ..base }).is_ok());
        assert!(check("/w", &Facts { now_ms: 16 * h, ..base }).is_ok());
        assert!(check("/w", &Facts { last_used_ms: None, ..base }).is_ok());
        // force never passes the session and main rules
        assert_eq!(code(&Facts { busy: true, force: true, ..base }).as_deref(), Some("artifacts.busy"));
        assert_eq!(code(&Facts { main: true, force: true, ..base }).as_deref(), Some("artifacts.main"));
    }

    #[test]
    fn quiet_hours_default_and_bounds() {
        use serde_json::json;
        assert_eq!(quiet_ms(&json!({})), 6 * 3_600_000);
        assert_eq!(quiet_ms(&json!({"janitor": {"artifact_quiet_hours": 2}})), 2 * 3_600_000);
        assert_eq!(quiet_ms(&json!({"janitor": {"artifact_quiet_hours": 0}})), 6 * 3_600_000);
        assert_eq!(quiet_ms(&json!({"janitor": {"artifact_quiet_hours": 9999}})), 6 * 3_600_000);
        assert_eq!(validate(&json!({}), &json!({"artifact_quiet_hours": 24})), Some(json!({"artifact_quiet_hours": 24})));
        assert_eq!(validate(&json!({}), &json!({"artifact_quiet_hours": 0})), None);
        assert_eq!(validate(&json!({}), &json!({"other": 1})), None);
    }

    /// The real tree: a link inside a reclaimed folder is unlinked, what it points to stays whole.
    #[test]
    fn os_remove_never_follows_a_link_out() {
        let d = std::env::temp_dir().join(format!("axl-art-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let (doomed, outside) = (d.join("target"), d.join("outside"));
        std::fs::create_dir_all(doomed.join("debug/deep")).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(doomed.join("debug/deep/a.bin"), vec![0u8; 4096]).unwrap();
        std::fs::write(outside.join("keep.txt"), "keep").unwrap();
        #[cfg(windows)]
        let made = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(doomed.join("debug/link"))
            .arg(&outside)
            .output()
            .is_ok_and(|o| o.status.success());
        #[cfg(not(windows))]
        let made = std::os::unix::fs::symlink(&outside, doomed.join("debug/link")).is_ok();
        let t = OsTree;
        assert!(t.size(&doomed) >= 4096);
        if made {
            assert!(t.is_link(&doomed.join("debug/link")));
            // the 4 bytes behind the link are not counted twice through it
            assert_eq!(t.size(&doomed), 4096);
        }
        t.remove(&doomed).unwrap();
        assert!(!doomed.exists());
        assert_eq!(std::fs::read_to_string(outside.join("keep.txt")).unwrap(), "keep");
        let _ = std::fs::remove_dir_all(&d);
    }
}
