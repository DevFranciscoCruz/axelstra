//! Free space of the volumes that hold the worktrees (`.axelstra-wt`), the `disk` settings and the pure planner that
//! picks which inactive worktrees lose their build artifacts when a volume runs low (see `server/disk.rs`).

use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// `settings.json` key `disk`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Config {
    pub enabled: bool,
    /// low at or below this many GB free...
    pub min_free_gb: u64,
    /// ...or at or below this percentage of the volume
    pub min_free_pct: u64,
    /// a reclaim goes on until free is above both lines plus this
    pub margin_gb: u64,
}

impl Default for Config {
    fn default() -> Self {
        Config { enabled: true, min_free_gb: 30, min_free_pct: 10, margin_gb: 10 }
    }
}

impl Config {
    /// From the settings; a missing or bad field keeps its default.
    pub fn from_settings(s: &Value) -> Config {
        let d = Config::default();
        let v = &s["disk"];
        Config {
            enabled: v["enabled"].as_bool().unwrap_or(d.enabled),
            min_free_gb: v["min_free_gb"].as_u64().filter(|n| *n <= 1 << 20).unwrap_or(d.min_free_gb),
            min_free_pct: v["min_free_pct"].as_u64().filter(|n| *n <= 90).unwrap_or(d.min_free_pct),
            margin_gb: v["margin_gb"].as_u64().filter(|n| *n <= 1 << 20).unwrap_or(d.margin_gb),
        }
    }

    pub fn to_json(self) -> Value {
        json!({"enabled": self.enabled, "min_free_gb": self.min_free_gb, "min_free_pct": self.min_free_pct, "margin_gb": self.margin_gb})
    }

    /// The line in MB for a volume of `total_mb`: the higher of the two.
    pub fn min_free_mb(&self, total_mb: u64) -> u64 {
        (self.min_free_gb * 1024).max(total_mb * self.min_free_pct / 100)
    }

    pub fn is_low(&self, free_mb: u64, total_mb: u64) -> bool {
        self.enabled && free_mb <= self.min_free_mb(total_mb)
    }

    /// How many MB a reclaim on a low volume must free to end above the line plus the margin (0 when not low).
    pub fn need_mb(&self, free_mb: u64, total_mb: u64) -> u64 {
        if !self.is_low(free_mb, total_mb) {
            return 0;
        }
        (self.min_free_mb(total_mb) + self.margin_gb * 1024 + 1).saturating_sub(free_mb)
    }
}

/// Validates a `disk` settings patch (the keys present change, the rest stays).
pub fn validate(saved: &Value, patch: &Value) -> Option<Value> {
    let mut out = Config::from_settings(saved).to_json();
    for (k, v) in patch.as_object()? {
        match k.as_str() {
            "enabled" => out[k] = json!(v.as_bool()?),
            "min_free_gb" | "margin_gb" => out[k] = json!(v.as_u64().filter(|n| *n <= 1 << 20)?),
            "min_free_pct" => out[k] = json!(v.as_u64().filter(|n| *n <= 90)?),
            _ => return None,
        }
    }
    Some(out)
}

/// One volume holding worktrees.
#[derive(Clone, Debug, PartialEq)]
pub struct Volume {
    /// its root (`C:\`), or on unix the first worktree folder seen on it
    pub root: String,
    pub free_mb: u64,
    pub total_mb: u64,
}

/// `(free, total)` bytes of the volume holding `path`.
#[cfg(windows)]
pub fn free_space(path: &Path) -> Option<(u64, u64)> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
    let (mut avail, mut total, mut _free) = (0u64, 0u64, 0u64);
    // SAFETY: a NUL-terminated path and three out pointers that live for the call
    let ok = unsafe { GetDiskFreeSpaceExW(wide.as_ptr(), &mut avail, &mut total, &mut _free) };
    (ok != 0).then_some((avail, total))
}

#[cfg(unix)]
pub fn free_space(path: &Path) -> Option<(u64, u64)> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: a NUL-terminated path and an out struct that lives for the call
    if unsafe { libc::statvfs(c.as_ptr(), &mut s) } != 0 {
        return None;
    }
    let frsize = if s.f_frsize > 0 { s.f_frsize as u64 } else { s.f_bsize as u64 };
    Some((s.f_bavail as u64 * frsize, s.f_blocks as u64 * frsize))
}

#[cfg(not(any(windows, unix)))]
pub fn free_space(_: &Path) -> Option<(u64, u64)> {
    None
}

/// What tells two folders apart by volume: the drive or share on Windows, the device on unix.
fn volume_key(path: &Path) -> Option<String> {
    #[cfg(windows)]
    {
        use std::path::Component;
        match path.components().next()? {
            Component::Prefix(p) => Some(p.as_os_str().to_string_lossy().to_uppercase()),
            _ => None,
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        std::fs::metadata(path).ok().map(|m| m.dev().to_string())
    }
    #[cfg(not(any(windows, unix)))]
    {
        Some(path.to_string_lossy().into_owned())
    }
}

/// The root shown for a volume.
fn volume_root(path: &Path) -> String {
    #[cfg(windows)]
    {
        if let Some(std::path::Component::Prefix(p)) = path.components().next() {
            return format!("{}\\", p.as_os_str().to_string_lossy());
        }
    }
    path.to_string_lossy().into_owned()
}

/// The `.axelstra-wt` folders of the projects (repo parent), existing ones only.
pub fn wt_dirs(project_paths: impl IntoIterator<Item = String>) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = vec![];
    for p in project_paths {
        if let Some(d) = Path::new(&p).parent().map(|d| d.join(".axelstra-wt")).filter(|d| d.is_dir()) {
            if !out.contains(&d) {
                out.push(d);
            }
        }
    }
    out
}

/// The volumes holding these folders, each once. `(volume, key)`; `key_of` tells which volume a path is on.
pub fn volumes(dirs: &[PathBuf]) -> Vec<(Volume, String)> {
    let mut out: Vec<(Volume, String)> = vec![];
    for d in dirs {
        let Some(key) = volume_key(d) else { continue };
        if out.iter().any(|(_, k)| *k == key) {
            continue;
        }
        let Some((free, total)) = free_space(d) else { continue };
        out.push((Volume { root: volume_root(d), free_mb: free / (1024 * 1024), total_mb: total / (1024 * 1024) }, key));
    }
    out
}

/// The volume key of a path (see `volumes`).
pub fn key_of(path: &Path) -> Option<String> {
    volume_key(path)
}

/// A worktree that may lose its artifacts, as the planner sees it.
#[derive(Clone, Debug, PartialEq)]
pub struct Cand {
    pub path: String,
    pub mb: u64,
    /// last use, ms since the epoch (0 when unknown: counts as oldest)
    pub last_used_ms: u64,
}

/// Which candidates to reclaim to free `need_mb`: the oldest first, the biggest among equally old, until the need is
/// met. Ones with nothing to free are left out. Returns indexes into `cands`.
pub fn plan(cands: &[Cand], need_mb: u64) -> Vec<usize> {
    if need_mb == 0 {
        return vec![];
    }
    let mut idx: Vec<usize> = (0..cands.len()).filter(|i| cands[*i].mb > 0).collect();
    idx.sort_by(|a, b| {
        let (a, b) = (&cands[*a], &cands[*b]);
        a.last_used_ms.cmp(&b.last_used_ms).then(b.mb.cmp(&a.mb)).then(a.path.cmp(&b.path))
    });
    let mut got = 0;
    let mut out = vec![];
    for i in idx {
        if got >= need_mb {
            break;
        }
        got += cands[i].mb;
        out.push(i);
    }
    out
}

/// `"ax/a 9.1 GB, ax/b 7.0 GB"`: the biggest worktrees, for the `disk.low` notice.
pub fn top_text(rows: &[(String, u64)], n: usize) -> String {
    let mut r: Vec<&(String, u64)> = rows.iter().filter(|r| r.1 > 0).collect();
    r.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    r.iter().take(n).map(|(name, mb)| format!("{name} {:.1} GB", *mb as f64 / 1024.0)).collect::<Vec<_>>().join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn low_is_either_line_and_the_need_reaches_the_margin() {
        let c = Config::default();
        // 1 TB volume: the 10% line (102.4 GB) is above 30 GB
        let tb = 1024 * 1024;
        assert_eq!(c.min_free_mb(tb), tb / 10);
        assert!(c.is_low(tb / 10, tb));
        assert!(!c.is_low(tb / 10 + 1, tb));
        // 100 GB volume: 30 GB is the higher line
        let small = 100 * 1024;
        assert_eq!(c.min_free_mb(small), 30 * 1024);
        assert_eq!(c.need_mb(20 * 1024, small), 20 * 1024 + 1);
        assert_eq!(c.need_mb(50 * 1024, small), 0);
        let off = Config { enabled: false, ..c };
        assert!(!off.is_low(0, small));
        assert_eq!(off.need_mb(0, small), 0);
    }

    #[test]
    fn config_defaults_and_validation() {
        assert_eq!(Config::from_settings(&json!({})), Config::default());
        let s = json!({"disk": {"enabled": false, "min_free_gb": 5, "min_free_pct": 200}});
        let c = Config::from_settings(&s);
        assert_eq!((c.enabled, c.min_free_gb, c.min_free_pct, c.margin_gb), (false, 5, 10, 10));
        let v = validate(&json!({}), &json!({"margin_gb": 2})).unwrap();
        assert_eq!(v["margin_gb"], 2);
        assert_eq!(v["min_free_gb"], 30);
        assert!(validate(&json!({}), &json!({"min_free_pct": 95})).is_none());
        assert!(validate(&json!({}), &json!({"bogus": true})).is_none());
        assert!(validate(&json!({}), &json!({"enabled": "yes"})).is_none());
    }

    #[test]
    fn the_plan_takes_the_oldest_then_biggest_until_the_need_is_met() {
        let c = |p: &str, mb: u64, at: u64| Cand { path: p.into(), mb, last_used_ms: at };
        let cands = vec![c("new", 9000, 300), c("old-small", 100, 100), c("old-big", 5000, 100), c("empty", 0, 0), c("mid", 4000, 200)];
        assert_eq!(plan(&cands, 0), Vec::<usize>::new());
        assert_eq!(plan(&cands, 3000), vec![2]);
        assert_eq!(plan(&cands, 5050), vec![2, 1]);
        assert_eq!(plan(&cands, 6000), vec![2, 1, 4]);
        // not enough anywhere: everything that frees something
        assert_eq!(plan(&cands, 1 << 30), vec![2, 1, 4, 0]);
    }

    #[test]
    fn the_top_text_names_the_biggest() {
        let rows = vec![("ax/a".to_string(), 9318), ("ax/b".to_string(), 7168), ("ax/c".to_string(), 10), ("ax/z".to_string(), 0)];
        assert_eq!(top_text(&rows, 2), "ax/a 9.1 GB, ax/b 7.0 GB");
    }

    #[test]
    fn the_temp_dir_has_a_volume() {
        let d = std::env::temp_dir();
        let (free, total) = free_space(&d).unwrap();
        assert!(total > 0 && free <= total);
        let v = volumes(&[d.clone(), d]);
        assert_eq!(v.len(), 1);
    }
}
