//! Log files and crash capture for the UI host: a daily-rolling `logs/axelstra.log.<date>` (bounded count),
//! a bounded `crash.log`, a panic hook that writes both, and the text bundle the user can paste in a bug report.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

const LOG_PREFIX: &str = "axelstra.log";
/// daily files kept in `logs/`
pub const KEEP_LOGS: usize = 7;
/// `crash.log` is cut down when it grows past this
pub const CRASH_MAX: u64 = 512 * 1024;
/// bytes of each file included in the diagnostics bundle
const TAIL_BYTES: u64 = 16 * 1024;

static GUARD: OnceLock<tracing_appender::non_blocking::WorkerGuard> = OnceLock::new();

pub fn logs_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("logs")
}

pub fn crash_path(data_dir: &Path) -> PathBuf {
    data_dir.join("crash.log")
}

/// The daily log files in `dir`, oldest first (the date suffix sorts lexicographically).
fn log_files(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with(LOG_PREFIX)))
        .collect();
    files.sort();
    files
}

/// Deletes the oldest daily logs so that at most `keep` remain.
pub fn prune_logs(dir: &Path, keep: usize) {
    let files = log_files(dir);
    for old in files.iter().take(files.len().saturating_sub(keep)) {
        let _ = std::fs::remove_file(old);
    }
}

pub fn latest_log(dir: &Path) -> Option<PathBuf> {
    log_files(dir).pop()
}

/// Shrinks `path` to its newest half (cut at a line start) when it is larger than `max` bytes.
pub fn bound_file(path: &Path, max: u64) {
    let Ok(meta) = std::fs::metadata(path) else { return };
    if meta.len() <= max {
        return;
    }
    let Ok(bytes) = std::fs::read(path) else { return };
    let mut start = bytes.len() - (max / 2) as usize;
    if let Some(nl) = bytes[start..].iter().position(|&b| b == b'\n') {
        start += nl + 1;
    }
    let _ = std::fs::write(path, &bytes[start..]);
}

/// Appends `text` to `crash.log`, keeping the file bounded.
pub fn append_crash(path: &Path, text: &str) {
    bound_file(path, CRASH_MAX);
    let _ = std::fs::OpenOptions::new().create(true).append(true).open(path).and_then(|mut f| f.write_all(text.as_bytes()));
}

/// Last `max` bytes of a text file (lossy), cut at a line start; empty when missing.
pub fn tail_file(path: &Path, max: u64) -> String {
    let Ok(bytes) = std::fs::read(path) else { return String::new() };
    let mut start = bytes.len().saturating_sub(max as usize);
    if start > 0 {
        if let Some(nl) = bytes[start..].iter().position(|&b| b == b'\n') {
            start += nl + 1;
        }
    }
    String::from_utf8_lossy(&bytes[start..]).into_owned()
}

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

pub fn panic_line(info: &dyn std::fmt::Display, backtrace: &dyn std::fmt::Display) -> String {
    format!("{} panic: {info}\n{backtrace}\n", now_secs())
}

/// Starts file logging and installs the panic hook. Logging failures never stop the app.
pub fn init(data_dir: &Path) {
    let dir = logs_dir(data_dir);
    let _ = std::fs::create_dir_all(&dir);
    prune_logs(&dir, KEEP_LOGS);
    let appender = tracing_appender::rolling::daily(&dir, LOG_PREFIX);
    let (writer, guard) = tracing_appender::non_blocking(appender);
    let _ = GUARD.set(guard);
    let _ = tracing_subscriber::fmt().with_writer(writer).with_ansi(false).with_max_level(tracing::Level::INFO).try_init();
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "axelstra started");

    let crash = crash_path(data_dir);
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let bt = std::backtrace::Backtrace::force_capture();
        tracing::error!("panic: {info}\n{bt}");
        append_crash(&crash, &panic_line(info, &bt));
        previous(info);
    }));
}

/// The text a webview error is logged as.
pub fn frontend_line(kind: &str, message: &str, stack: &str) -> String {
    let clip = |s: &str, n: usize| s.chars().take(n).collect::<String>();
    format!("frontend {}: {}\n{}", clip(kind, 40), clip(message, 2000), clip(stack, 8000))
}

pub fn log_frontend(kind: &str, message: &str, stack: &str) {
    tracing::error!("{}", frontend_line(kind, message, stack));
}

/// Text for a bug report: versions, paths, and the tail of the newest log and `crash.log`.
pub fn bundle(data_dir: &Path) -> String {
    let logs = logs_dir(data_dir);
    let latest = latest_log(&logs);
    let crash = crash_path(data_dir);
    format!(
        "axelstra {}\nos: {} {}\ndata_dir: {}\n\n== {} ==\n{}\n== crash.log ==\n{}\n",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH,
        data_dir.display(),
        latest.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "no log file".into()),
        latest.map(|p| tail_file(&p, TAIL_BYTES)).unwrap_or_default(),
        tail_file(&crash, TAIL_BYTES),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("axelstra-logging-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn prune_keeps_newest_logs_only() {
        let d = tmp("prune");
        for day in 1..=10 {
            std::fs::write(d.join(format!("{LOG_PREFIX}.2026-01-{day:02}")), "x").unwrap();
        }
        std::fs::write(d.join("other.txt"), "x").unwrap();
        prune_logs(&d, 7);
        let left = log_files(&d);
        assert_eq!(left.len(), 7);
        assert!(left[0].ends_with(format!("{LOG_PREFIX}.2026-01-04")));
        assert!(d.join("other.txt").exists());
        assert!(latest_log(&d).unwrap().ends_with(format!("{LOG_PREFIX}.2026-01-10")));
    }

    #[test]
    fn crash_log_stays_bounded_and_keeps_newest() {
        let d = tmp("crash");
        let p = d.join("crash.log");
        let line = "x".repeat(99) + "\n";
        let big = line.repeat(CRASH_MAX as usize / 100 + 50);
        std::fs::write(&p, big).unwrap();
        append_crash(&p, "newest panic\n");
        let len = std::fs::metadata(&p).unwrap().len();
        assert!(len <= CRASH_MAX / 2 + 200, "{len}");
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(text.ends_with("newest panic\n"));
        assert_eq!(text.lines().next().unwrap().len(), 99);
    }

    #[test]
    fn small_crash_log_is_untouched() {
        let d = tmp("small");
        let p = d.join("crash.log");
        append_crash(&p, "a\n");
        append_crash(&p, "b\n");
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "a\nb\n");
    }

    #[test]
    fn tail_cuts_at_line_start() {
        let d = tmp("tail");
        let p = d.join("f");
        std::fs::write(&p, "aaaa\nbbbb\ncccc\n").unwrap();
        assert_eq!(tail_file(&p, 7), "cccc\n");
        assert_eq!(tail_file(&p, 1000), "aaaa\nbbbb\ncccc\n");
        assert_eq!(tail_file(&d.join("missing"), 10), "");
    }

    #[test]
    fn bundle_has_version_paths_and_tails() {
        let d = tmp("bundle");
        std::fs::create_dir_all(logs_dir(&d)).unwrap();
        std::fs::write(logs_dir(&d).join(format!("{LOG_PREFIX}.2026-01-01")), "old line\n").unwrap();
        std::fs::write(logs_dir(&d).join(format!("{LOG_PREFIX}.2026-01-02")), "new line\n").unwrap();
        std::fs::write(crash_path(&d), "boom\n").unwrap();
        let b = bundle(&d);
        assert!(b.contains(env!("CARGO_PKG_VERSION")));
        assert!(b.contains(&d.display().to_string()));
        assert!(b.contains("new line") && !b.contains("old line"));
        assert!(b.contains("boom"));
    }

    #[test]
    fn frontend_line_is_clipped() {
        let l = frontend_line("error", &"m".repeat(5000), "stack");
        assert!(l.starts_with("frontend error: mmm"));
        assert!(l.len() < 2100);
    }
}
