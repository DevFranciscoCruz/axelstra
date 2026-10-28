//! The build environment every session the daemon starts carries, so many agents building at once do not exhaust the
//! machine. All decisions are pure functions (facts and settings are passed in) and the only moment they run is a
//! spawn: no timer, no watcher. Nothing here compiles anything and a variable the user already set is never touched.
//!
//! Settings (`settings.json`, additive, under a `build` object; read and written with the existing `settings.get/set`):
//! - `enabled` (default true): off = no variable at all from this module.
//! - `jobs` (default null = derived): fixed `CARGO_BUILD_JOBS`.
//! - `sccache` (`"auto"` default | `"off"`): `auto` sets `RUSTC_WRAPPER` only when `sccache` is found on PATH. Never required.
//! - `shared_target` (default false): one `CARGO_TARGET_DIR` per project, `<data_dir>/target/<project-id>`. Risks, so it
//!   is off: cargo's build-directory lock serializes every build of that project across its worktrees; fingerprints
//!   include the path, so workspace crates of a different worktree path still rebuild (only dependencies are shared);
//!   and worktrees on different profiles/features thrash the same artifacts.
//! - `shared_js_cache` (default true): only the package manager's download cache goes to a shared folder under the data
//!   dir (`npm_config_cache`, pnpm `npm_config_store_dir`, `BUN_INSTALL_CACHE_DIR`), chosen by the lockfile in the
//!   session's folder, so the user's manager and the `node_modules` layout are untouched.
//!
//! The resource governor (branch `ax/resource-governor`) can feed its free-memory limit into [`Facts::free_mem_bytes`]
//! and [`lock_budget_mb`]; this module does not depend on it.

use serde_json::Value;
use std::path::{Path, PathBuf};

/// Peak memory of one rustc job (a link of the daemon or the host is the worst one). Same as `scripts/lib/jobs.mjs`.
pub const DEFAULT_PER_JOB_BYTES: u64 = 3 * 1024 * 1024 * 1024 / 2;
/// Left for the OS and the editor.
pub const DEFAULT_RESERVE_BYTES: u64 = 2 * 1024 * 1024 * 1024;
/// Granularity of the lock's memory budget: a unit is one lock file.
pub const LOCK_UNIT_MB: u64 = 512;

/// How many parallel compile jobs fit: half the cores (SMT siblings do not help a compile), limited by free memory.
/// Never fewer than 1. Unknown free memory (`None`) limits nothing but the cores.
pub fn jobs_for(cores: usize, free_mem_bytes: Option<u64>, per_job_bytes: u64, reserve_bytes: u64) -> u32 {
    let by_cpu = (cores.max(1) / 2).max(1) as u64;
    let by_mem = match free_mem_bytes {
        Some(free) => free.saturating_sub(reserve_bytes) / per_job_bytes.max(1),
        None => by_cpu,
    };
    by_cpu.min(by_mem).max(1) as u32
}

#[derive(Debug, Clone, PartialEq)]
pub struct BuildSettings {
    pub enabled: bool,
    pub jobs: Option<u32>,
    pub sccache: bool,
    pub shared_target: bool,
    pub shared_js_cache: bool,
    pub lock_budget_mb: Option<u64>,
}

impl Default for BuildSettings {
    fn default() -> Self {
        BuildSettings { enabled: true, jobs: None, sccache: true, shared_target: false, shared_js_cache: true, lock_budget_mb: None }
    }
}

impl BuildSettings {
    /// From the whole settings document; anything missing or of the wrong type keeps its default.
    pub fn from_settings(settings: &Value) -> Self {
        let b = &settings["build"];
        let d = BuildSettings::default();
        BuildSettings {
            enabled: b["enabled"].as_bool().unwrap_or(d.enabled),
            jobs: b["jobs"].as_u64().filter(|j| *j > 0).map(|j| j.min(256) as u32),
            sccache: b["sccache"].as_str().map(|m| m != "off").unwrap_or(d.sccache),
            shared_target: b["shared_target"].as_bool().unwrap_or(d.shared_target),
            shared_js_cache: b["shared_js_cache"].as_bool().unwrap_or(d.shared_js_cache),
            lock_budget_mb: b["lock_budget_mb"].as_u64().filter(|m| *m > 0),
        }
    }
}

/// What the machine and the session's folder look like right now (injected, so tests need no real machine).
#[derive(Debug, Clone, Default)]
pub struct Facts {
    pub cores: usize,
    pub free_mem_bytes: Option<u64>,
    /// `sccache` found on PATH, if any
    pub sccache: Option<PathBuf>,
    /// file names present in the session's folder that tell the JS package manager (`pnpm-lock.yaml`, ...)
    pub files: Vec<String>,
    pub data_dir: PathBuf,
}

/// First `name` (also `name.exe`) in a PATH-like list. `exists` is injected.
pub fn find_on_path(path: &std::ffi::OsStr, name: &str, exists: impl Fn(&Path) -> bool) -> Option<PathBuf> {
    for dir in std::env::split_paths(path) {
        for file in [name.to_string(), format!("{name}.exe")] {
            let p = dir.join(file);
            if exists(&p) {
                return Some(p);
            }
        }
    }
    None
}

/// `sccache` on this PATH, optional.
pub fn detect_sccache(path: &std::ffi::OsStr, exists: impl Fn(&Path) -> bool) -> Option<PathBuf> {
    find_on_path(path, "sccache", exists)
}

/// Folder names are made of the project id; anything outside `[A-Za-z0-9_-]` becomes `_`.
fn safe_name(id: &str) -> String {
    let s: String = id.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
    s.trim_matches('_').chars().take(64).collect()
}

/// The variables to add to a new session. `user_has(key)` says a variable is already set by the user (their
/// environment or the spawn request): such a key is skipped, never overridden. `project` is the project id.
pub fn build_env(s: &BuildSettings, project: Option<&str>, f: &Facts, user_has: impl Fn(&str) -> bool) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    if !s.enabled {
        return out;
    }
    let mut put = |k: &str, v: String| {
        if !user_has(k) && !out.iter().any(|(ek, _)| ek == k) {
            out.push((k.to_string(), v));
        }
    };
    let jobs = s.jobs.unwrap_or_else(|| jobs_for(f.cores, f.free_mem_bytes, DEFAULT_PER_JOB_BYTES, DEFAULT_RESERVE_BYTES));
    put("CARGO_BUILD_JOBS", jobs.to_string());
    if s.sccache && !user_has("RUSTC_WRAPPER") && !user_has("CARGO_BUILD_RUSTC_WRAPPER") {
        if let Some(p) = &f.sccache {
            put("RUSTC_WRAPPER", p.to_string_lossy().into_owned());
            // sccache does not cache incremental compilations
            put("CARGO_INCREMENTAL", "0".into());
        }
    }
    if s.shared_target {
        if let Some(name) = project.map(safe_name).filter(|n| !n.is_empty()) {
            put("CARGO_TARGET_DIR", f.data_dir.join("target").join(name).to_string_lossy().into_owned());
        }
    }
    if s.shared_js_cache {
        let has = |n: &str| f.files.iter().any(|x| x == n);
        let cache = f.data_dir.join("cache");
        let dir = |n: &str| cache.join(n).to_string_lossy().into_owned();
        if has("pnpm-lock.yaml") {
            put("npm_config_store_dir", dir("pnpm-store"));
        } else if has("bun.lock") || has("bun.lockb") {
            put("BUN_INSTALL_CACHE_DIR", dir("bun"));
        } else if has("package-lock.json") {
            put("npm_config_cache", dir("npm"));
        }
    }
    out
}

/// Total and available physical memory in bytes, `None` when this platform cannot say.
pub fn memory() -> Option<(u64, u64)> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
        // SAFETY: a zeroed MEMORYSTATUSEX with dwLength set is the documented input
        let mut m: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
        m.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
        if unsafe { GlobalMemoryStatusEx(&mut m) } == 0 {
            return None;
        }
        Some((m.ullTotalPhys, m.ullAvailPhys))
    }
    #[cfg(target_os = "linux")]
    {
        parse_meminfo(&std::fs::read_to_string("/proc/meminfo").ok()?)
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        None
    }
}

/// `(total, available)` bytes from the text of `/proc/meminfo`.
pub fn parse_meminfo(text: &str) -> Option<(u64, u64)> {
    let kb = |key: &str| -> Option<u64> {
        let l = text.lines().find(|l| l.starts_with(key))?;
        Some(l.split_whitespace().nth(1)?.parse::<u64>().ok()? * 1024)
    };
    Some((kb("MemTotal:")?, kb("MemAvailable:")?))
}

/// Facts for a session started in `cwd`, read now.
pub fn facts_now(cwd: Option<&Path>, data_dir: &Path) -> Facts {
    let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
    let sccache = std::env::var_os("PATH").and_then(|p| detect_sccache(&p, |f| f.is_file()));
    let files = cwd
        .map(|d| {
            ["pnpm-lock.yaml", "bun.lock", "bun.lockb", "package-lock.json"]
                .iter()
                .filter(|n| d.join(n).is_file())
                .map(|n| n.to_string())
                .collect()
        })
        .unwrap_or_default();
    Facts { cores, free_mem_bytes: memory().map(|(_, avail)| avail), sccache, files, data_dir: data_dir.to_path_buf() }
}

/// The memory budget of `axelstra lock --mem-mb`, in MB: the setting, else the physical memory minus the reserve.
/// At least one unit; `None` total memory (unknown platform) gives 8 GB.
pub fn lock_budget_mb(setting: Option<u64>, total_bytes: Option<u64>) -> u64 {
    let mb = setting.unwrap_or_else(|| match total_bytes {
        Some(t) => t.saturating_sub(DEFAULT_RESERVE_BYTES) / (1024 * 1024),
        None => 8 * 1024,
    });
    mb.max(LOCK_UNIT_MB)
}

/// Estimated memory of a heavy command with `jobs` compile jobs, in MB, for `lock --mem-mb` defaults.
pub fn mem_mb_for_jobs(jobs: u32) -> u64 {
    jobs.max(1) as u64 * DEFAULT_PER_JOB_BYTES / (1024 * 1024)
}

/// Lock units a holder of `need_mb` takes, out of `units` in all (at least 1, never more than the budget holds, so a
/// command bigger than the budget still runs, alone).
pub fn units_for(need_mb: u64, units: usize) -> usize {
    (need_mb.div_ceil(LOCK_UNIT_MB).max(1) as usize).min(units.max(1))
}

/// The pure admission step of the memory lock: `busy[i]` says unit `i` is held. Returns the `need` lowest free units
/// when that many are free, else the first busy unit to wait on (its release is the event to re-evaluate on), so a
/// waiter sleeps in the OS lock call and never polls. Holders cannot deadlock: nobody holds units while waiting.
pub fn admit(busy: &[bool], need: usize) -> Result<Vec<usize>, usize> {
    let free: Vec<usize> = (0..busy.len()).filter(|i| !busy[*i]).collect();
    if free.len() >= need {
        return Ok(free.into_iter().take(need).collect());
    }
    Err(busy.iter().position(|b| *b).unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const GB: u64 = 1024 * 1024 * 1024;

    fn facts() -> Facts {
        Facts { cores: 16, free_mem_bytes: Some(32 * GB), sccache: None, files: vec![], data_dir: PathBuf::from("/d") }
    }
    fn get<'a>(v: &'a [(String, String)], k: &str) -> Option<&'a str> {
        v.iter().find(|(x, _)| x == k).map(|(_, v)| v.as_str())
    }
    fn none(_: &str) -> bool {
        false
    }

    #[test]
    fn jobs_follow_the_js_formula() {
        let j = |c, m: Option<u64>| jobs_for(c, m, DEFAULT_PER_JOB_BYTES, DEFAULT_RESERVE_BYTES);
        assert_eq!(j(16, Some(32 * GB)), 8);
        assert_eq!(j(16, Some(8 * GB)), 4);
        assert_eq!(j(16, Some(0)), 1);
        assert_eq!(j(16, Some(GB)), 1);
        assert_eq!(j(1, Some(64 * GB)), 1);
        assert_eq!(j(2, Some(64 * GB)), 1);
        assert_eq!(j(0, None), 1);
        assert_eq!(j(16, None), 8);
        assert_eq!(jobs_for(4, Some(64 * GB), 8 * GB, DEFAULT_RESERVE_BYTES), 2);
    }

    #[test]
    fn settings_default_and_junk() {
        assert_eq!(BuildSettings::from_settings(&json!({})), BuildSettings::default());
        assert_eq!(BuildSettings::from_settings(&json!({"build": "x"})), BuildSettings::default());
        let s = BuildSettings::from_settings(
            &json!({"build": {"enabled": false, "jobs": 3, "sccache": "off", "shared_target": true, "shared_js_cache": false, "lock_budget_mb": 4096}}),
        );
        assert_eq!(
            s,
            BuildSettings {
                enabled: false,
                jobs: Some(3),
                sccache: false,
                shared_target: true,
                shared_js_cache: false,
                lock_budget_mb: Some(4096)
            }
        );
        assert_eq!(BuildSettings::from_settings(&json!({"build": {"jobs": 0}})).jobs, None);
    }

    #[test]
    fn sccache_is_found_on_path_or_not_at_all() {
        let path = std::env::join_paths(["/a", "/b"]).unwrap();
        let hit = detect_sccache(&path, |p| p == Path::new("/b").join("sccache"));
        assert_eq!(hit, Some(Path::new("/b").join("sccache")));
        assert_eq!(detect_sccache(&path, |_| false), None);
    }

    #[test]
    fn default_env_is_only_the_jobs_and_nothing_else() {
        let v = build_env(&BuildSettings::default(), Some("p1"), &facts(), none);
        assert_eq!(v, vec![("CARGO_BUILD_JOBS".to_string(), "8".to_string())]);
    }

    #[test]
    fn disabled_adds_nothing_and_a_jobs_setting_is_used() {
        let off = BuildSettings { enabled: false, ..Default::default() };
        assert!(build_env(&off, None, &facts(), none).is_empty());
        let s = BuildSettings { jobs: Some(2), ..Default::default() };
        assert_eq!(get(&build_env(&s, None, &facts(), none), "CARGO_BUILD_JOBS"), Some("2"));
    }

    #[test]
    fn sccache_sets_the_wrapper_and_turns_incremental_off() {
        let mut f = facts();
        f.sccache = Some(PathBuf::from("/bin/sccache"));
        let v = build_env(&BuildSettings::default(), None, &f, none);
        assert_eq!(get(&v, "RUSTC_WRAPPER"), Some("/bin/sccache"));
        assert_eq!(get(&v, "CARGO_INCREMENTAL"), Some("0"));
        let off = BuildSettings { sccache: false, ..Default::default() };
        assert_eq!(get(&build_env(&off, None, &f, none), "RUSTC_WRAPPER"), None);
        // the user's own wrapper wins, and then incremental stays as it is
        let v = build_env(&BuildSettings::default(), None, &f, |k| k == "RUSTC_WRAPPER");
        assert_eq!(get(&v, "RUSTC_WRAPPER"), None);
        assert_eq!(get(&v, "CARGO_INCREMENTAL"), None);
    }

    #[test]
    fn nothing_the_user_set_is_overridden() {
        let mut f = facts();
        f.sccache = Some(PathBuf::from("/bin/sccache"));
        f.files = vec!["package-lock.json".into()];
        let s = BuildSettings { shared_target: true, ..Default::default() };
        let v = build_env(&s, Some("p"), &f, |_| true);
        assert!(v.is_empty());
        let v = build_env(&s, Some("p"), &f, |k| k == "CARGO_BUILD_JOBS" || k == "CARGO_INCREMENTAL");
        assert_eq!(get(&v, "CARGO_BUILD_JOBS"), None);
        assert_eq!(get(&v, "CARGO_INCREMENTAL"), None);
        assert!(get(&v, "CARGO_TARGET_DIR").is_some());
    }

    #[test]
    fn shared_target_is_per_project_and_off_by_default() {
        let s = BuildSettings { shared_target: true, ..Default::default() };
        let v = build_env(&s, Some("my proj/1"), &facts(), none);
        assert_eq!(get(&v, "CARGO_TARGET_DIR").map(PathBuf::from), Some(Path::new("/d").join("target").join("my_proj_1")));
        assert_eq!(get(&build_env(&s, None, &facts(), none), "CARGO_TARGET_DIR"), None);
        assert_eq!(get(&build_env(&BuildSettings::default(), Some("p"), &facts(), none), "CARGO_TARGET_DIR"), None);
    }

    #[test]
    fn js_cache_follows_the_lockfile_and_never_changes_the_manager() {
        let with = |file: &str| {
            let mut f = facts();
            f.files = vec![file.into()];
            build_env(&BuildSettings::default(), None, &f, none)
        };
        assert!(get(&with("pnpm-lock.yaml"), "npm_config_store_dir").is_some());
        assert!(get(&with("bun.lock"), "BUN_INSTALL_CACHE_DIR").is_some());
        assert!(get(&with("bun.lockb"), "BUN_INSTALL_CACHE_DIR").is_some());
        assert!(get(&with("package-lock.json"), "npm_config_cache").is_some());
        assert_eq!(get(&with("package-lock.json"), "npm_config_store_dir"), None);
        let off = BuildSettings { shared_js_cache: false, ..Default::default() };
        let mut f = facts();
        f.files = vec!["pnpm-lock.yaml".into()];
        assert_eq!(get(&build_env(&off, None, &f, none), "npm_config_store_dir"), None);
    }

    #[test]
    fn meminfo_text_is_read() {
        let t = "MemTotal:       16000000 kB\nMemFree:  100 kB\nMemAvailable:    8000000 kB\n";
        assert_eq!(parse_meminfo(t), Some((16000000 * 1024, 8000000 * 1024)));
        assert_eq!(parse_meminfo("nope"), None);
    }

    #[test]
    fn lock_budget_and_units() {
        assert_eq!(lock_budget_mb(Some(4096), Some(64 * GB)), 4096);
        assert_eq!(lock_budget_mb(None, Some(16 * GB)), 14 * 1024);
        assert_eq!(lock_budget_mb(None, Some(GB)), LOCK_UNIT_MB);
        assert_eq!(lock_budget_mb(None, None), 8192);
        assert_eq!(mem_mb_for_jobs(2), 3072);
        assert_eq!(mem_mb_for_jobs(0), 1536);
        assert_eq!(units_for(0, 8), 1);
        assert_eq!(units_for(512, 8), 1);
        assert_eq!(units_for(513, 8), 2);
        assert_eq!(units_for(99999, 8), 8);
    }

    #[test]
    fn admission_takes_free_units_or_names_the_one_to_wait_on() {
        assert_eq!(admit(&[false, false, false], 2), Ok(vec![0, 1]));
        assert_eq!(admit(&[true, false, true, false], 2), Ok(vec![1, 3]));
        assert_eq!(admit(&[true, false, true], 2), Err(0));
        assert_eq!(admit(&[false, true, true], 2), Err(1));
        // a holder taking the whole budget waits for any busy unit
        assert_eq!(admit(&[false, true], 2), Err(1));
        assert_eq!(admit(&[true, true], 1), Err(0));
    }

    #[test]
    fn budget_never_admits_more_than_it_holds() {
        // 8 units, holders of 3 units each: two fit, the third waits
        let mut busy = vec![false; 8];
        for _ in 0..2 {
            for i in admit(&busy, 3).unwrap() {
                busy[i] = true;
            }
        }
        assert!(admit(&busy, 3).is_err());
        assert!(admit(&busy, 2).is_ok());
    }
}
