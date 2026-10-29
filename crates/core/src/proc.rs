//! The one place that decides whether a child process may show a console window.
//!
//! The daemon and the desktop app have no console, so a console child (git, gh, docker, robocopy, ssh, wsl, cmd...)
//! would flash a new window every time it starts. Call [`hidden`] on every `Command` they spawn themselves.
//!
//! Deliberately NOT hidden: the user's terminal and agent ptys (`portable-pty` spawns in `pty.rs`, they are the visible
//! terminal), and the build `axelstra lock` runs in the user's own terminal (it already goes through `Job::spawn`, which
//! hides only the extra window). Tests that spawn helper processes are left alone too.

use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

/// Commands prepared through [`hidden`] (about the children the process started); read by `debug.load`.
static SPAWNS: AtomicU64 = AtomicU64::new(0);

pub fn spawn_count() -> u64 {
    SPAWNS.load(Ordering::Relaxed)
}

static FIRST: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();

/// Starts the clock `spawn_stats` measures against (the first `hidden` call does it otherwise).
pub fn mark_start() {
    FIRST.get_or_init(std::time::Instant::now);
}

/// (children started through [`hidden`], seconds since the clock started): the rate is the caller's division.
pub fn spawn_stats() -> (u64, u64) {
    (spawn_count(), FIRST.get().map(|t| t.elapsed().as_secs()).unwrap_or(0))
}

/// Directories the process asks the OS to watch for changes right now (each one is a kernel handle on Windows).
static WATCHES: AtomicU64 = AtomicU64::new(0);

pub fn watch_count() -> u64 {
    WATCHES.load(Ordering::Relaxed)
}

/// Counts watched directories while it lives: `add` for each one registered, all given back on drop.
#[derive(Default)]
pub struct WatchTally(u64);

impl WatchTally {
    pub fn new(n: usize) -> WatchTally {
        let mut t = WatchTally(0);
        t.add(n);
        t
    }

    pub fn add(&mut self, n: usize) {
        self.0 += n as u64;
        WATCHES.fetch_add(n as u64, Ordering::Relaxed);
    }
}

impl Drop for WatchTally {
    fn drop(&mut self) {
        WATCHES.fetch_sub(self.0, Ordering::Relaxed);
    }
}

/// `CREATE_NO_WINDOW`: the child gets no console window. Combine it with other flags only through `creation_flags`
/// (that call replaces, it does not add: the desktop app starts the daemon with `DETACHED_PROCESS | NO_WINDOW`).
pub const NO_WINDOW: u32 = 0x0800_0000;

/// Keeps a console window from flashing on Windows; a no-op elsewhere. Do not call it after another `creation_flags`.
pub fn hidden(cmd: &mut Command) -> &mut Command {
    mark_start();
    SPAWNS.fetch_add(1, Ordering::Relaxed);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(NO_WINDOW);
    }
    cmd
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn a_hidden_child_still_runs_and_reports_its_exit_code() {
        let ok = hidden(Command::new("cmd").args(["/d", "/c", "exit 0"])).status().unwrap();
        assert!(ok.success());
        let bad = hidden(Command::new("cmd").args(["/d", "/c", "exit 3"])).status().unwrap();
        assert_eq!(bad.code(), Some(3));
    }

    #[test]
    fn a_hidden_child_keeps_its_piped_output() {
        let o = hidden(Command::new("cmd").args(["/d", "/c", "echo hi"])).output().unwrap();
        assert!(String::from_utf8_lossy(&o.stdout).contains("hi"));
    }
}
