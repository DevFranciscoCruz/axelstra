//! Plumbing shared by the daemon's state files (sessions.json, settings.json, projects.json) and its locks.

use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};

/// `lock()` that survives a poisoned mutex. A handler panic is caught (`catch_unwind` in the connection loop), and
/// the maps behind the registry locks stay consistent between statements, so the next caller carries on with the data.
pub trait LockRecover<T> {
    fn lock_recover(&self) -> MutexGuard<'_, T>;
}

impl<T> LockRecover<T> for Mutex<T> {
    fn lock_recover(&self) -> MutexGuard<'_, T> {
        self.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// How many rotating copies (`<name>.1` newest .. `<name>.3` oldest) `write_backed_up` keeps.
const BACKUPS: usize = 3;

/// Current schema version of the files written with `write_json`. Bump it and add an arm to `migrate` when a
/// persisted struct changes shape incompatibly.
pub const CURRENT_VERSION: u32 = 1;

static NOTICES: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Records a message about lost or recovered state for the server/UI to show later (and logs it).
pub fn notice(msg: String) {
    eprintln!("{msg}");
    NOTICES.lock_recover().push(msg);
}

/// Takes (and clears) the notices recorded since the last call.
pub fn take_notices() -> Vec<String> {
    std::mem::take(&mut *NOTICES.lock_recover())
}

/// Runs `op`, retrying a few times with short sleeps on Windows: an antivirus or indexer holding the target briefly
/// makes a rename fail with a sharing violation (PermissionDenied).
fn retry_op(mut op: impl FnMut() -> io::Result<()>) -> io::Result<()> {
    let mut attempt = 0u64;
    loop {
        match op() {
            Err(e) if cfg!(windows) && attempt < 6 && is_sharing_violation(&e) => {
                attempt += 1;
                std::thread::sleep(std::time::Duration::from_millis(10 * attempt));
            }
            other => return other,
        }
    }
}

fn is_sharing_violation(e: &io::Error) -> bool {
    e.kind() == io::ErrorKind::PermissionDenied || matches!(e.raw_os_error(), Some(5 | 32 | 33))
}

fn rename_retry(from: &Path, to: &Path) -> io::Result<()> {
    retry_op(|| std::fs::rename(from, to))
}

fn sync_dir(dir: &Path) {
    #[cfg(unix)]
    if let Ok(d) = std::fs::File::open(dir) {
        let _ = d.sync_all();
    }
    #[cfg(not(unix))]
    let _ = dir;
}

/// Writes `bytes` to a temp file next to `path`, fsyncs it and renames it over (then fsyncs the folder on unix):
/// a crash, a power cut or a second writer never leaves a half-written or empty file. Creates the parent folder.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let dir = path.parent().filter(|d| !d.as_os_str().is_empty());
    if let Some(dir) = dir {
        std::fs::create_dir_all(dir)?;
    }
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let tmp = path.with_file_name(format!("{name}.{}.{}.tmp", std::process::id(), TMP_SEQ.fetch_add(1, Ordering::Relaxed)));
    let res = (|| {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        drop(f);
        rename_retry(&tmp, path)
    })();
    match res {
        Ok(()) => {
            if let Some(dir) = dir {
                sync_dir(dir);
            }
            Ok(())
        }
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

fn backup_path(path: &Path, n: usize) -> PathBuf {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    path.with_file_name(format!("{name}.{n}"))
}

/// `write_atomic` that first rotates the previous contents into `<name>.1` (older ones shift to `.2`, `.3`), so a
/// bad save can be recovered from. A failed rotation never blocks the save.
pub fn write_backed_up(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if path.is_file() {
        for n in (1..BACKUPS).rev() {
            let from = backup_path(path, n);
            if from.is_file() {
                let _ = std::fs::remove_file(backup_path(path, n + 1));
                let _ = rename_retry(&from, &backup_path(path, n + 1));
            }
        }
        let _ = std::fs::remove_file(backup_path(path, 1));
        let _ = std::fs::copy(path, backup_path(path, 1));
    }
    write_atomic(path, bytes)
}

/// Saves `value` in the versioned envelope `{"v":N,"data":...}` with rotating backups.
pub fn write_json<T: Serialize + ?Sized>(path: &Path, value: &T) -> io::Result<()> {
    let env = serde_json::json!({"v": CURRENT_VERSION, "data": value});
    let bytes = serde_json::to_vec_pretty(&env).map_err(io::Error::other)?;
    write_backed_up(path, &bytes)
}

/// Upgrades `data` one version step (`from` -> `from + 1`). v0 is the legacy unversioned layout; v0 -> v1 only wraps
/// it, so the data is unchanged.
fn migrate(from: u32, data: Value) -> Result<Value, String> {
    match from {
        0 => Ok(data),
        v => Err(format!("no migration from version {v}")),
    }
}

/// Parses the text of a state file: envelope or legacy bare value, migrated to the current version.
fn parse_state<T: DeserializeOwned>(text: &str) -> Result<T, String> {
    let raw: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let (mut ver, mut data) = match raw {
        Value::Object(mut m) if m.len() == 2 && m.get("v").is_some_and(Value::is_u64) && m.contains_key("data") => {
            (m["v"].as_u64().unwrap_or(0) as u32, m.remove("data").unwrap_or(Value::Null))
        }
        other => (0, other),
    };
    if ver > CURRENT_VERSION {
        return Err(format!("written by a newer version (v{ver}, this one understands up to v{CURRENT_VERSION})"));
    }
    while ver < CURRENT_VERSION {
        data = migrate(ver, data)?;
        ver += 1;
    }
    serde_json::from_value(data).map_err(|e| e.to_string())
}

/// Whether `text` is a state file this version can read (envelope or legacy bare value), without touching any file.
pub fn validate_state(text: &str) -> Result<(), String> {
    parse_state::<Value>(text).map(|_| ())
}

/// Where a file that could not be parsed is kept: `projects.json.bad.<unix-timestamp>`, with a counter appended if
/// that name is taken, so an earlier one is never overwritten.
fn bad_path(path: &Path) -> PathBuf {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let mut cand = path.with_file_name(format!("{name}.bad.{ts}"));
    let mut n = 1;
    while cand.exists() {
        cand = path.with_file_name(format!("{name}.bad.{ts}-{n}"));
        n += 1;
    }
    cand
}

/// Reads a JSON state file (envelope or legacy bare value). `Ok(None)` when it does not exist, or when it is corrupt
/// or incompatible and no backup is usable: the file is moved to `.bad.<ts>`, a notice is recorded
/// (`take_notices`) and the next save cannot overwrite it with an empty default. When a backup (`.1`..`.3`, newest
/// first) parses, it is restored and returned. `Err` only when the file exists but cannot be read, in which case the
/// caller must not write an empty default over it.
pub fn read_json<T: DeserializeOwned>(path: &Path) -> io::Result<Option<T>> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    let err = match parse_state(&text) {
        Ok(v) => return Ok(Some(v)),
        Err(e) => e,
    };
    let bad = bad_path(path);
    let kept =
        if rename_retry(path, &bad).is_ok() { format!("moved to {}", bad.display()) } else { "could not be moved aside".to_string() };
    for n in 1..=BACKUPS {
        let bp = backup_path(path, n);
        let Ok(btext) = std::fs::read_to_string(&bp) else { continue };
        if let Ok(v) = parse_state::<T>(&btext) {
            let _ = write_atomic(path, btext.as_bytes());
            notice(format!("{} is unusable ({err}); {kept}; restored from backup {}", path.display(), bp.display()));
            return Ok(Some(v));
        }
    }
    notice(format!("{} is unusable ({err}); {kept}; starting empty", path.display()));
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("axl-store-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn bads(d: &Path, name: &str) -> Vec<PathBuf> {
        let pre = format!("{name}.bad.");
        let mut v: Vec<PathBuf> = std::fs::read_dir(d)
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.file_name().unwrap().to_string_lossy().starts_with(&pre))
            .collect();
        v.sort();
        v
    }

    #[test]
    fn atomic_write_replaces_and_leaves_no_temp_file() {
        let d = dir("atomic");
        let f = d.join("sub").join("state.json");
        write_atomic(&f, b"one").unwrap();
        write_atomic(&f, b"two").unwrap();
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "two");
        let left: Vec<_> = std::fs::read_dir(f.parent().unwrap()).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(left.len(), 1, "{left:?}");
        write_atomic(&f, b"").unwrap();
        assert_eq!(std::fs::read(&f).unwrap(), b"");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn corrupt_file_is_moved_aside_not_lost() {
        let d = dir("corrupt");
        let f = d.join("projects.json");
        std::fs::write(&f, "{ not json").unwrap();
        let _ = take_notices();
        assert_eq!(read_json::<Value>(&f).unwrap(), None);
        assert!(!f.exists());
        let b = bads(&d, "projects.json");
        assert_eq!(b.len(), 1);
        assert_eq!(std::fs::read_to_string(&b[0]).unwrap(), "{ not json");
        assert!(take_notices().iter().any(|n| n.contains("projects.json") && n.contains("starting empty")));
        // a later save starts a fresh file and the broken one is still there
        write_atomic(&f, b"[]").unwrap();
        assert_eq!(bads(&d, "projects.json").len(), 1);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn missing_file_reads_as_none_and_valid_file_as_value() {
        let d = dir("missing");
        let f = d.join("settings.json");
        assert_eq!(read_json::<Value>(&f).unwrap(), None);
        write_atomic(&f, br#"{"a":1}"#).unwrap();
        assert_eq!(read_json::<Value>(&f).unwrap(), Some(json!({"a": 1})));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn unreadable_file_is_an_error_not_an_empty_default() {
        let d = dir("unreadable");
        // a directory where the file should be: exists, cannot be read as text
        let f = d.join("projects.json");
        std::fs::create_dir_all(&f).unwrap();
        assert!(read_json::<Value>(&f).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn write_json_wraps_in_versioned_envelope_and_reads_back() {
        let d = dir("envelope");
        let f = d.join("projects.json");
        write_json(&f, &json!([1, 2])).unwrap();
        let raw: Value = serde_json::from_str(&std::fs::read_to_string(&f).unwrap()).unwrap();
        assert_eq!(raw, json!({"v": CURRENT_VERSION, "data": [1, 2]}));
        assert_eq!(read_json::<Vec<u32>>(&f).unwrap(), Some(vec![1, 2]));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn legacy_unversioned_file_still_loads() {
        let d = dir("legacy");
        let f = d.join("projects.json");
        std::fs::write(&f, "[1,2,3]").unwrap();
        assert_eq!(read_json::<Vec<u32>>(&f).unwrap(), Some(vec![1, 2, 3]));
        assert!(bads(&d, "projects.json").is_empty());
        // an object that merely has a "v" key is data, not an envelope
        std::fs::write(&f, r#"{"v":1,"x":2}"#).unwrap();
        assert_eq!(read_json::<Value>(&f).unwrap(), Some(json!({"v": 1, "x": 2})));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn newer_version_is_incompatible_and_kept_aside_with_notice() {
        let d = dir("newer");
        let f = d.join("automations.json");
        std::fs::write(&f, format!(r#"{{"v":{},"data":[]}}"#, CURRENT_VERSION + 1)).unwrap();
        let _ = take_notices();
        assert_eq!(read_json::<Vec<u32>>(&f).unwrap(), None);
        assert!(!f.exists());
        assert_eq!(bads(&d, "automations.json").len(), 1);
        assert!(take_notices().iter().any(|n| n.contains("automations.json") && n.contains("newer")));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn wrong_shape_is_incompatible() {
        let d = dir("shape");
        let f = d.join("projects.json");
        write_json(&f, &json!({"not": "a list"})).unwrap();
        assert_eq!(read_json::<Vec<u32>>(&f).unwrap(), None);
        assert_eq!(bads(&d, "projects.json").len(), 1);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn second_corrupt_file_does_not_overwrite_the_first_bad() {
        let d = dir("twobad");
        let f = d.join("projects.json");
        std::fs::write(&f, "first {").unwrap();
        assert_eq!(read_json::<Value>(&f).unwrap(), None);
        std::fs::write(&f, "second {").unwrap();
        assert_eq!(read_json::<Value>(&f).unwrap(), None);
        let b = bads(&d, "projects.json");
        assert_eq!(b.len(), 2, "{b:?}");
        let mut texts: Vec<String> = b.iter().map(|p| std::fs::read_to_string(p).unwrap()).collect();
        texts.sort();
        assert_eq!(texts, ["first {", "second {"]);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn backups_rotate_and_keep_three() {
        let d = dir("rotate");
        let f = d.join("sessions.json");
        for i in 1..=6 {
            write_backed_up(&f, format!("v{i}").as_bytes()).unwrap();
        }
        let read = |p: PathBuf| std::fs::read_to_string(p).unwrap();
        assert_eq!(read(f.clone()), "v6");
        assert_eq!(read(backup_path(&f, 1)), "v5");
        assert_eq!(read(backup_path(&f, 2)), "v4");
        assert_eq!(read(backup_path(&f, 3)), "v3");
        assert!(!backup_path(&f, 4).exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn corrupt_main_falls_back_to_newest_valid_backup() {
        let d = dir("fallback");
        let f = d.join("projects.json");
        write_json(&f, &json!([1])).unwrap();
        write_json(&f, &json!([2])).unwrap();
        write_json(&f, &json!([3])).unwrap();
        // main and the newest backup are broken: .2 holds the newest valid copy
        std::fs::write(&f, "garbage").unwrap();
        std::fs::write(backup_path(&f, 1), "garbage too").unwrap();
        let _ = take_notices();
        assert_eq!(read_json::<Vec<u32>>(&f).unwrap(), Some(vec![1]));
        assert_eq!(bads(&d, "projects.json").len(), 1);
        assert!(take_notices().iter().any(|n| n.contains("restored from backup")));
        // the main file was restored, so the next read is clean
        assert_eq!(read_json::<Vec<u32>>(&f).unwrap(), Some(vec![1]));
        assert_eq!(bads(&d, "projects.json").len(), 1);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn sharing_violations_are_retried_on_windows_only() {
        let mut calls = 0;
        let r = retry_op(|| {
            calls += 1;
            if calls < 3 {
                Err(io::Error::from(io::ErrorKind::PermissionDenied))
            } else {
                Ok(())
            }
        });
        if cfg!(windows) {
            assert!(r.is_ok());
            assert_eq!(calls, 3);
        } else {
            assert!(r.is_err());
            assert_eq!(calls, 1);
        }
        // other errors are never retried
        let mut n = 0;
        let r = retry_op(|| {
            n += 1;
            Err(io::Error::from(io::ErrorKind::NotFound))
        });
        assert!(r.is_err());
        assert_eq!(n, 1);
    }

    #[test]
    fn poisoned_lock_still_locks() {
        let m = std::sync::Arc::new(Mutex::new(5));
        let m2 = m.clone();
        let _ = std::thread::spawn(move || {
            let _g = m2.lock().unwrap();
            panic!("handler bug");
        })
        .join();
        assert!(m.is_poisoned());
        *m.lock_recover() += 1;
        assert_eq!(*m.lock_recover(), 6);
    }
}
