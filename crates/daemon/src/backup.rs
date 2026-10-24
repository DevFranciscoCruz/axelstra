//! Export and restore of the daemon's state files (`backup.export` / `backup.restore`).
//!
//! A backup is a folder under `<data_dir>/backups/` holding verbatim copies of the state files: sessions, projects,
//! settings and automations. The token file and everything else in the data dir (logs, locks, scrollback, notes) is
//! never copied. A restore validates every file before it writes any, and goes through `write_backed_up`, so the files
//! it replaces stay recoverable as `<name>.1`.

use crate::store::{validate_state, write_backed_up};
use anyhow::Result;
use axelstra_core::coded;
use std::path::{Path, PathBuf};

/// The files a backup holds: the whole of the state that is not derived from git or from the agents' own logs.
pub const FILES: [&str; 4] = ["sessions.json", "projects.json", "settings.json", "automations.json"];

fn regular(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|m| m.is_file())
}

/// Copies the state files of `data_dir` into a new folder `backups/backup-<unix seconds>` and returns it.
pub fn export(data_dir: &Path) -> Result<PathBuf> {
    let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let root = data_dir.join("backups");
    let mut dest = root.join(format!("backup-{ts}"));
    let mut n = 1;
    while dest.exists() {
        dest = root.join(format!("backup-{ts}-{n}"));
        n += 1;
    }
    let present: Vec<&str> = FILES.into_iter().filter(|f| regular(&data_dir.join(f))).collect();
    if present.is_empty() {
        return Err(coded!("backup.nothing", "there is no saved state to back up yet"));
    }
    std::fs::create_dir_all(&dest)
        .map_err(|e| coded!("backup.create_failed", "could not create {path}: {detail}", path = dest.display(), detail = e))?;
    for f in present {
        std::fs::copy(data_dir.join(f), dest.join(f))
            .map_err(|e| coded!("backup.copy_failed", "could not copy {file}: {detail}", file = f, detail = e))?;
    }
    Ok(dest)
}

/// Puts the state files found in the backup folder `src` back into `data_dir` and returns their names. Nothing is
/// written unless every file in the folder parses as a state file of this version or older.
pub fn restore(data_dir: &Path, src: &Path) -> Result<Vec<String>> {
    if !src.is_dir() {
        return Err(coded!("backup.not_folder", "{path} is not a backup folder", path = src.display()));
    }
    let mut found: Vec<(&str, Vec<u8>)> = vec![];
    for f in FILES {
        let p = src.join(f);
        if !regular(&p) {
            continue;
        }
        let bytes = std::fs::read(&p)
            .map_err(|e| coded!("backup.read_failed", "could not read {path}: {detail}", path = p.display(), detail = e))?;
        let text = std::str::from_utf8(&bytes).map_err(|_| coded!("backup.not_text", "{file} is not text", file = f))?;
        validate_state(text).map_err(|e| coded!("backup.unusable", "{file} in the backup is unusable: {detail}", file = f, detail = e))?;
        found.push((f, bytes));
    }
    if found.is_empty() {
        return Err(coded!("backup.none_found", "{path} has none of {files}", path = src.display(), files = FILES.join(", ")));
    }
    let mut restored = vec![];
    for (f, bytes) in found {
        write_backed_up(&data_dir.join(f), &bytes)
            .map_err(|e| coded!("backup.write_failed", "could not write {file}: {detail}", file = f, detail = e))?;
        restored.push(f.to_string());
    }
    Ok(restored)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_carry_message_codes() {
        let d = dir("codes");
        let c = |e: anyhow::Error| axelstra_core::msg::find(&e).map(|c| c.code.clone()).unwrap_or_default();
        assert_eq!(c(export(&d).unwrap_err()), "backup.nothing");
        assert_eq!(c(restore(&d, &d.join("missing")).unwrap_err()), "backup.not_folder");
        assert_eq!(c(restore(&d, &d).unwrap_err()), "backup.none_found");
        std::fs::write(d.join("projects.json"), "not json").unwrap();
        let src = d.join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("projects.json"), "not json").unwrap();
        assert_eq!(c(restore(&d, &src).unwrap_err()), "backup.unusable");
        let _ = std::fs::remove_dir_all(&d);
    }

    fn dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("axl-backup-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn export_copies_state_files_and_never_the_token() {
        let d = dir("export");
        std::fs::write(d.join("projects.json"), r#"{"v":1,"data":[]}"#).unwrap();
        std::fs::write(d.join("settings.json"), r#"{"v":1,"data":{"terse":"short"}}"#).unwrap();
        std::fs::write(d.join("token"), "secret").unwrap();
        std::fs::write(d.join("crash.log"), "x").unwrap();
        let out = export(&d).unwrap();
        let mut names: Vec<String> =
            std::fs::read_dir(&out).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        assert_eq!(names, ["projects.json", "settings.json"]);
        assert!(out.starts_with(d.join("backups")));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn export_with_no_state_is_an_error() {
        let d = dir("empty");
        assert!(export(&d).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn restore_round_trips_and_keeps_the_replaced_file() {
        let d = dir("roundtrip");
        std::fs::write(d.join("settings.json"), r#"{"v":1,"data":{"terse":"short"}}"#).unwrap();
        let b = export(&d).unwrap();
        std::fs::write(d.join("settings.json"), r#"{"v":1,"data":{"terse":"tiny"}}"#).unwrap();
        assert_eq!(restore(&d, &b).unwrap(), ["settings.json"]);
        assert!(std::fs::read_to_string(d.join("settings.json")).unwrap().contains("short"));
        assert!(std::fs::read_to_string(d.join("settings.json.1")).unwrap().contains("tiny"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn restore_writes_nothing_when_any_file_is_bad() {
        let d = dir("bad");
        let b = d.join("bk");
        std::fs::create_dir_all(&b).unwrap();
        std::fs::write(b.join("projects.json"), r#"{"v":1,"data":[]}"#).unwrap();
        std::fs::write(b.join("settings.json"), "{ not json").unwrap();
        std::fs::write(d.join("projects.json"), r#"{"v":1,"data":["keep"]}"#).unwrap();
        assert!(restore(&d, &b).is_err());
        assert!(std::fs::read_to_string(d.join("projects.json")).unwrap().contains("keep"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn restore_refuses_a_missing_or_foreign_folder() {
        let d = dir("foreign");
        assert!(restore(&d, &d.join("nope")).is_err());
        let other = d.join("other");
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(other.join("token"), "x").unwrap();
        assert!(restore(&d, &other).is_err());
        assert!(!d.join("token").exists());
        let _ = std::fs::remove_dir_all(&d);
    }
}
