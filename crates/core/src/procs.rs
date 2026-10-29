//! Process tree lookups: which processes descend from a session's shell.

use crate::sync::LockRecover;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// One process of the table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Proc {
    pub pid: u32,
    pub ppid: u32,
    /// process group id (unix only; 0 on Windows)
    pub pgid: u32,
    /// resident memory in KiB (unix only; 0 on Windows, where Job Objects report memory themselves)
    pub rss_kib: u64,
}

/// A copy of the machine's process table. Taking one costs a `ps -A` on unix, so take it once per poll and pass it
/// to every lookup (`with_descendants_in`, `Job::usage_in`, ...) instead of letting each lookup dump the table.
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub procs: Vec<Proc>,
}

impl Snapshot {
    /// `roots` and every process below them.
    pub fn descendants(&self, roots: &[u32]) -> HashSet<u32> {
        let table: Vec<(u32, u32)> = self.procs.iter().map(|p| (p.pid, p.ppid)).collect();
        descend(&table, roots)
    }

    /// Resident KiB of the processes whose group id is in `groups`.
    pub fn rss_of_groups(&self, groups: &[i32]) -> u64 {
        self.procs.iter().filter(|p| groups.contains(&(p.pgid as i32))).map(|p| p.rss_kib).sum()
    }
}

#[cfg(windows)]
fn read_table() -> Vec<Proc> {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::*;
    let mut out = Vec::new();
    unsafe {
        let h = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if h == INVALID_HANDLE_VALUE {
            return out;
        }
        let mut e: PROCESSENTRY32W = std::mem::zeroed();
        e.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut ok = Process32FirstW(h, &mut e);
        while ok != 0 {
            out.push(Proc { pid: e.th32ProcessID, ppid: e.th32ParentProcessID, pgid: 0, rss_kib: 0 });
            ok = Process32NextW(h, &mut e);
        }
        CloseHandle(h);
    }
    out
}

#[cfg(unix)]
fn read_table() -> Vec<Proc> {
    let Ok(o) = std::process::Command::new("ps").args(["-A", "-o", "pid=,ppid=,pgid=,rss="]).output() else { return Vec::new() };
    String::from_utf8_lossy(&o.stdout)
        .lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            Some(Proc {
                pid: it.next()?.parse().ok()?,
                ppid: it.next()?.parse().ok()?,
                pgid: it.next()?.parse().ok()?,
                rss_kib: it.next()?.parse().ok()?,
            })
        })
        .collect()
}

/// A fresh process table.
pub fn snapshot() -> Snapshot {
    Snapshot { procs: read_table() }
}

/// A table at most `max_age` old, shared by every caller in the process: N sessions polled together cost one dump.
pub fn snapshot_cached(max_age: Duration) -> Arc<Snapshot> {
    static CACHE: Mutex<Option<(Instant, Arc<Snapshot>)>> = Mutex::new(None);
    // the lock is held while dumping on purpose: concurrent callers wait for the one dump instead of each making their own
    let mut g = CACHE.lock_recover();
    if let Some((at, s)) = g.as_ref() {
        if at.elapsed() <= max_age {
            return s.clone();
        }
    }
    let s = Arc::new(snapshot());
    *g = Some((Instant::now(), s.clone()));
    s
}

/// `roots` and every process below them (takes a fresh snapshot; see `with_descendants_in` to share one).
pub fn with_descendants(roots: &[u32]) -> HashSet<u32> {
    snapshot().descendants(roots)
}

/// Same, on a snapshot the caller already has.
pub fn with_descendants_in(snap: &Snapshot, roots: &[u32]) -> HashSet<u32> {
    snap.descendants(roots)
}

/// Pids of `pids` that are still running.
pub fn alive_of(pids: &HashSet<u32>) -> HashSet<u32> {
    let snap = snapshot();
    snap.procs.iter().map(|p| p.pid).filter(|p| pids.contains(p)).collect()
}

/// Kills each of `pids` (not their children). For leftovers of a session's tree that outlived the kill, taken a moment
/// before it so a reused number is not a concern.
pub fn kill_pids(pids: &HashSet<u32>) {
    for &pid in pids {
        #[cfg(windows)]
        unsafe {
            use windows_sys::Win32::Foundation::CloseHandle;
            use windows_sys::Win32::System::Threading::{OpenProcess, TerminateProcess, PROCESS_TERMINATE};
            let h = OpenProcess(PROCESS_TERMINATE, 0, pid);
            if !h.is_null() {
                TerminateProcess(h, 1);
                CloseHandle(h);
            }
        }
        #[cfg(unix)]
        unsafe {
            libc::kill(pid as i32, libc::SIGKILL);
        }
    }
}

/// Environment variable every session's process carries. Its value is `<daemon pid>:<session id>:<spawn time>`, and
/// children inherit it, so a process that left the session's tree (`setsid`, a double fork, a reparent to init) can
/// still be found by it.
pub const MARK_ENV: &str = "AXELSTRA_MARK";

/// The `AXELSTRA_MARK` entry prefix that names every process of session `id` under daemon `daemon_pid`, whatever its
/// spawn time (a woken session keeps its id and gets a new time).
pub fn mark_prefix(daemon_pid: u32, id: u32) -> String {
    format!("{MARK_ENV}={daemon_pid}:{id}:")
}

/// Whether a NUL-separated environment block has an entry starting with `prefix`.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn environ_has(block: &[u8], prefix: &str) -> bool {
    block.split(|b| *b == 0).any(|e| e.starts_with(prefix.as_bytes()))
}

/// Pids whose environment carries an entry starting with `prefix` (see `mark_prefix`), other than this process.
/// Linux only (`/proc/<pid>/environ`): other systems give no way to read another process' environment, so they get an
/// empty set and keep relying on the process tree. Processes of other users are unreadable and skipped.
pub fn marked(prefix: &str) -> HashSet<u32> {
    #[allow(unused_mut)]
    let mut out = HashSet::new();
    #[cfg(target_os = "linux")]
    if let Ok(dir) = std::fs::read_dir("/proc") {
        let me = std::process::id();
        for e in dir.flatten() {
            let Some(pid) = e.file_name().to_str().and_then(|n| n.parse::<u32>().ok()) else { continue };
            if pid == me {
                continue;
            }
            if let Ok(block) = std::fs::read(e.path().join("environ")) {
                if environ_has(&block, prefix) {
                    out.insert(pid);
                }
            }
        }
    }
    #[cfg(not(target_os = "linux"))]
    let _ = prefix;
    out
}

/// Executable names (lowercase, no directory, no `.exe`) of those of `pids` that are running now. One table dump, so
/// ask for all the pids of a tree together.
pub fn names_of(pids: &HashSet<u32>) -> HashMap<u32, String> {
    let clean = |n: &str| {
        let n = n.rsplit(['/', '\\']).next().unwrap_or(n).to_ascii_lowercase();
        n.strip_suffix(".exe").map(str::to_string).unwrap_or(n)
    };
    let mut out = HashMap::new();
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
        use windows_sys::Win32::System::Diagnostics::ToolHelp::*;
        let h = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if h == INVALID_HANDLE_VALUE {
            return out;
        }
        let mut e: PROCESSENTRY32W = std::mem::zeroed();
        e.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut ok = Process32FirstW(h, &mut e);
        while ok != 0 {
            if pids.contains(&e.th32ProcessID) {
                let len = e.szExeFile.iter().position(|c| *c == 0).unwrap_or(e.szExeFile.len());
                out.insert(e.th32ProcessID, clean(&String::from_utf16_lossy(&e.szExeFile[..len])));
            }
            ok = Process32NextW(h, &mut e);
        }
        CloseHandle(h);
    }
    #[cfg(unix)]
    if let Ok(o) = std::process::Command::new("ps").args(["-A", "-o", "pid=,comm="]).output() {
        for l in String::from_utf8_lossy(&o.stdout).lines() {
            if let Some((pid, name)) = l.trim_start().split_once(char::is_whitespace) {
                if let Ok(pid) = pid.parse::<u32>() {
                    if pids.contains(&pid) {
                        out.insert(pid, clean(name.trim()));
                    }
                }
            }
        }
    }
    out
}

fn descend(table: &[(u32, u32)], roots: &[u32]) -> HashSet<u32> {
    let mut kids: HashMap<u32, Vec<u32>> = HashMap::new();
    for &(pid, ppid) in table {
        kids.entry(ppid).or_default().push(pid);
    }
    let mut seen: HashSet<u32> = roots.iter().copied().collect();
    let mut stack: Vec<u32> = roots.to_vec();
    while let Some(p) = stack.pop() {
        for &c in kids.get(&p).into_iter().flatten() {
            if seen.insert(c) {
                stack.push(c);
            }
        }
    }
    seen
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn walks_the_tree_and_survives_cycles() {
        let t = [(2, 1), (3, 2), (4, 2), (5, 99), (1, 4)];
        let got = descend(&t, &[2]);
        assert_eq!(got, HashSet::from([1, 2, 3, 4]));
    }

    #[test]
    fn own_process_is_in_the_snapshot() {
        assert!(with_descendants(&[std::process::id()]).contains(&std::process::id()));
        assert!(snapshot().procs.iter().any(|p| p.pid == std::process::id()));
    }

    #[test]
    fn cached_snapshot_is_shared_within_its_age() {
        let a = snapshot_cached(Duration::from_secs(60));
        let b = snapshot_cached(Duration::from_secs(60));
        assert!(Arc::ptr_eq(&a, &b));
        assert!(a.procs.iter().any(|p| p.pid == std::process::id()));
    }

    #[test]
    fn mark_prefix_matches_one_session_only() {
        let block = b"PATH=/bin\0AXELSTRA_MARK=7:12:1700\0HOME=/h\0";
        assert!(environ_has(block, &mark_prefix(7, 12)));
        assert!(!environ_has(block, &mark_prefix(7, 1)), "id 1 must not match id 12");
        assert!(!environ_has(block, &mark_prefix(8, 12)), "another daemon's session");
        assert!(!environ_has(b"", &mark_prefix(7, 12)));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn finds_a_process_that_left_the_tree() {
        use std::os::unix::process::CommandExt;
        let tag = format!("{}:{}:", std::process::id(), 900_000 + std::process::id() % 1000);
        // its own process group: `kill(-pgid)` of the session's group would not reach it
        // the kernel fills /proc/<pid>/environ a moment after spawn returns, so wait until the child says it runs
        let mut c = std::process::Command::new("sh");
        c.args(["-c", "echo up; sleep 30"]).env(MARK_ENV, format!("{tag}1")).process_group(0).stdout(std::process::Stdio::piped());
        let mut child = c.spawn().unwrap();
        let pid = child.id();
        let mut up = String::new();
        std::io::BufRead::read_line(&mut std::io::BufReader::new(child.stdout.take().unwrap()), &mut up).unwrap();
        assert_eq!(up.trim(), "up");
        let found = marked(&format!("{MARK_ENV}={tag}"));
        let _ = child.kill();
        let _ = child.wait();
        assert!(found.contains(&pid));
        assert!(!found.contains(&std::process::id()));
    }

    #[test]
    fn rss_sums_only_the_given_groups() {
        let s = Snapshot {
            procs: vec![
                Proc { pid: 1, ppid: 0, pgid: 10, rss_kib: 5 },
                Proc { pid: 2, ppid: 1, pgid: 10, rss_kib: 7 },
                Proc { pid: 3, ppid: 1, pgid: 20, rss_kib: 100 },
            ],
        };
        assert_eq!(s.rss_of_groups(&[10]), 12);
        assert_eq!(s.rss_of_groups(&[]), 0);
    }
}
