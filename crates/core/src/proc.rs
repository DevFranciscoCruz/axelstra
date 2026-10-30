//! The one place that decides whether a child process may show a console window.
//!
//! The daemon and the desktop app have no console, so a console child (git, gh, docker, robocopy, ssh, wsl, cmd...)
//! would flash a new window every time it starts. Call [`hidden`] on every `Command` they spawn themselves.
//!
//! Deliberately NOT hidden: the user's terminal and agent ptys (`portable-pty` spawns in `pty.rs`, they are the visible
//! terminal), and the build `axelstra lock` runs in the user's own terminal (it already goes through `Job::spawn`, which
//! hides only the extra window). Tests that spawn helper processes are left alone too.

use std::cell::RefCell;
use std::collections::HashMap;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

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

thread_local! {
    /// What the current thread is doing (a daemon method, a background loop): the "site" its children are counted under.
    static SITE: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Names the work of this thread until the guard drops (the previous name comes back): `hidden` counts the children
/// started meanwhile under it. A thread that never names its work counts under `other`.
pub fn site(name: &str) -> SiteGuard {
    SiteGuard(SITE.with(|s| s.replace(Some(name.to_string()))))
}

pub struct SiteGuard(Option<String>);

impl Drop for SiteGuard {
    fn drop(&mut self) {
        let prev = self.0.take();
        SITE.with(|s| *s.borrow_mut() = prev);
    }
}

/// Children of one (site, command), all time and per whole minute since the clock started.
#[derive(Default, Clone, Copy)]
struct Tally {
    total: u64,
    minute: u64,
    this_min: u64,
    last_min: u64,
}

impl Tally {
    /// Moves the minute buckets up to `minute` (a gap of more than one minute leaves an empty last minute).
    fn roll(&mut self, minute: u64) {
        if minute != self.minute {
            self.last_min = if minute == self.minute + 1 { self.this_min } else { 0 };
            self.this_min = 0;
            self.minute = minute;
        }
    }
}

/// Distinct (site, command) pairs kept; more fold into one `other` row, so a stream of odd names cannot grow it.
const MAX_SITES: usize = 256;

static SITES: Mutex<Option<HashMap<(String, String), Tally>>> = Mutex::new(None);

/// `git status`, `gh pr`, `powershell`: the program's name and, for git/gh/docker, the subcommand (options skipped).
fn command_key(cmd: &Command) -> String {
    let prog = std::path::Path::new(cmd.get_program()).file_stem().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default();
    if !matches!(prog.as_str(), "git" | "gh" | "docker") {
        return prog;
    }
    let mut args = cmd.get_args().map(|a| a.to_string_lossy());
    let mut words = vec![prog];
    while let Some(a) = args.next() {
        if a == "-c" || a == "-C" {
            args.next();
        } else if !a.starts_with('-') {
            words.push(a.to_string());
            // `gh pr list`, `git worktree list`: one more word names what the subcommand does
            if let Some(b) = args.next().filter(|b| !b.starts_with('-')) {
                if matches!(words[1].as_str(), "worktree" | "pr" | "run" | "api" | "repo" | "container" | "image" | "volume") {
                    words.push(b.to_string());
                }
            }
            break;
        }
    }
    words.join(" ")
}

fn minute_now() -> u64 {
    FIRST.get().map(|t| t.elapsed().as_secs() / 60).unwrap_or(0)
}

fn count_site(cmd: &Command) {
    let site = SITE.with(|s| s.borrow().clone()).unwrap_or_else(|| "other".to_string());
    let key = (site, command_key(cmd));
    let minute = minute_now();
    let mut g = SITES.lock().unwrap_or_else(|e| e.into_inner());
    let m = g.get_or_insert_with(HashMap::new);
    let key = if m.len() >= MAX_SITES && !m.contains_key(&key) { ("other".to_string(), "other".to_string()) } else { key };
    let t = m.entry(key).or_default();
    t.roll(minute);
    t.total += 1;
    t.this_min += 1;
}

/// One row of [`site_stats`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SiteStat {
    pub site: String,
    pub command: String,
    pub total: u64,
    /// children started in the last whole minute
    pub last_min: u64,
}

/// Children per (site, command), most in the last whole minute first, then most overall.
pub fn site_stats() -> Vec<SiteStat> {
    let minute = minute_now();
    let mut g = SITES.lock().unwrap_or_else(|e| e.into_inner());
    let mut v: Vec<SiteStat> = g
        .get_or_insert_with(HashMap::new)
        .iter_mut()
        .map(|((site, command), t)| {
            t.roll(minute);
            SiteStat { site: site.clone(), command: command.clone(), total: t.total, last_min: t.last_min }
        })
        .collect();
    v.sort_by(|a, b| b.last_min.cmp(&a.last_min).then(b.total.cmp(&a.total)).then(a.site.cmp(&b.site)).then(a.command.cmp(&b.command)));
    v
}

/// Keeps a console window from flashing on Windows; a no-op elsewhere. Do not call it after another `creation_flags`.
/// Counts the child under the thread's [`site`] (call it once the program and arguments are set).
pub fn hidden(cmd: &mut Command) -> &mut Command {
    mark_start();
    SPAWNS.fetch_add(1, Ordering::Relaxed);
    count_site(cmd);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(NO_WINDOW);
    }
    cmd
}

#[cfg(test)]
mod site_tests {
    use super::*;

    #[test]
    fn the_command_key_names_the_subcommand_and_skips_options() {
        let mut c = Command::new("git");
        c.args(["-c", "core.longpaths=true", "worktree", "list", "--porcelain"]);
        assert_eq!(command_key(&c), "git worktree list");
        let mut c = Command::new("C:/Program Files/Git/cmd/git.exe");
        c.args(["-c", "a=b", "rev-parse", "--git-dir"]);
        assert_eq!(command_key(&c), "git rev-parse");
        let mut c = Command::new("gh");
        c.args(["pr", "list", "--state", "merged"]);
        assert_eq!(command_key(&c), "gh pr list");
        assert_eq!(command_key(Command::new("powershell.exe").args(["-NoLogo"])), "powershell");
    }

    #[test]
    fn children_count_under_the_site_of_their_thread() {
        std::thread::spawn(|| {
            let _g = site("test.site_a");
            let mut c = Command::new("git");
            c.args(["status"]);
            count_site(&c);
            count_site(&c);
            {
                let _inner = site("test.site_b");
                count_site(&c);
            }
            count_site(&c);
        })
        .join()
        .unwrap();
        let stats = site_stats();
        let of = |s: &str| stats.iter().find(|r| r.site == s && r.command == "git status").map(|r| r.total);
        assert_eq!((of("test.site_a"), of("test.site_b")), (Some(3), Some(1)));
    }

    #[test]
    fn the_last_minute_is_the_previous_whole_minute() {
        let mut t = Tally::default();
        t.roll(0);
        t.this_min = 5;
        t.roll(1);
        assert_eq!((t.last_min, t.this_min), (5, 0));
        t.this_min = 2;
        t.roll(3);
        assert_eq!(t.last_min, 0);
    }
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
