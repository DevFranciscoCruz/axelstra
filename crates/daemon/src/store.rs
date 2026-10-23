//! Plumbing shared by the daemon's state files (sessions.json, settings.json, projects.json) and its locks.

use serde::de::DeserializeOwned;
use std::io;
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

/// Writes `bytes` to a temp file next to `path` and renames it over: a crash or a second writer never leaves
/// a half-written file. Creates the parent folder.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let tmp = path.with_file_name(format!("{name}.{}.{}.tmp", std::process::id(), TMP_SEQ.fetch_add(1, Ordering::Relaxed)));
    let res = std::fs::write(&tmp, bytes).and_then(|_| std::fs::rename(&tmp, path));
    if res.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    res
}

/// Where a file that could not be parsed is kept (`projects.json` -> `projects.json.bad`).
pub fn bad_path(path: &Path) -> PathBuf {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    path.with_file_name(format!("{name}.bad"))
}

/// Reads a JSON state file. `Ok(None)` when it does not exist, or when it is corrupt: a corrupt file is moved to
/// `.bad` so the next save cannot overwrite it with an empty default. `Err` only when the file exists but cannot be
/// read, in which case the caller must not write an empty default over it.
pub fn read_json<T: DeserializeOwned>(path: &Path) -> io::Result<Option<T>> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    match serde_json::from_str(&text) {
        Ok(v) => Ok(Some(v)),
        Err(e) => {
            let bad = bad_path(path);
            eprintln!("{} is corrupt ({e}); moved to {}", path.display(), bad.display());
            let _ = std::fs::rename(path, &bad);
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    fn dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("axl-store-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
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
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn corrupt_file_is_moved_aside_not_lost() {
        let d = dir("corrupt");
        let f = d.join("projects.json");
        std::fs::write(&f, "{ not json").unwrap();
        assert_eq!(read_json::<Value>(&f).unwrap(), None);
        assert!(!f.exists());
        assert_eq!(std::fs::read_to_string(bad_path(&f)).unwrap(), "{ not json");
        // a later save starts a fresh file and the broken one is still there
        write_atomic(&f, b"[]").unwrap();
        assert!(bad_path(&f).exists());
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
