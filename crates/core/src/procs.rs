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
    /// resident memory in KiB (working set on Windows; 0 for a process the OS will not let us open)
    pub rss_kib: u64,
}

/// A copy of the machine's process table. Taking one costs a single `ps -A` on unix (one batched call, also on macOS) or an open per process on Windows, so take it once per poll and pass it
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
            out.push(Proc { pid: e.th32ProcessID, ppid: e.th32ParentProcessID, pgid: 0, rss_kib: win_rss_kib(e.th32ProcessID) });
            ok = Process32NextW(h, &mut e);
        }
        CloseHandle(h);
    }
    out
}

/// Working set of `pid` in KiB; 0 when the process is gone or the OS refuses (idle, system and protected processes).
#[cfg(windows)]
fn win_rss_kib(pid: u32) -> u64 {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
    if pid == 0 || pid == 4 {
        return 0;
    }
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return 0;
        }
        let mut c: PROCESS_MEMORY_COUNTERS = std::mem::zeroed();
        c.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
        let kib = if GetProcessMemoryInfo(h, &mut c, c.cb) != 0 { c.WorkingSetSize as u64 / 1024 } else { 0 };
        CloseHandle(h);
        kib
    }
}

/// One `ps -A` for the whole table (BSD/macOS and procps both accept these columns).
#[cfg(unix)]
fn read_table() -> Vec<Proc> {
    let Ok(o) = std::process::Command::new("ps").args(["-A", "-o", "pid=,ppid=,pgid=,rss="]).output() else { return Vec::new() };
    parse_ps_table(&String::from_utf8_lossy(&o.stdout))
}

/// Parses `ps -o pid=,ppid=,pgid=,rss=` output; a line that does not have four numbers is skipped.
pub fn parse_ps_table(text: &str) -> Vec<Proc> {
    text.lines()
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

/// Parses `ps -o pid=,comm=` output. `comm` is a full path on macOS and may hold spaces, so the rest of the line is the name.
pub fn parse_ps_names(text: &str) -> Vec<(u32, String)> {
    text.lines()
        .filter_map(|l| {
            let (pid, name) = l.trim_start().split_once(char::is_whitespace)?;
            let name = name.trim();
            (!name.is_empty()).then_some((pid.parse().ok()?, name.to_string()))
        })
        .collect()
}

/// Lowercase executable name without directory (either separator) and without `.exe`.
pub fn clean_name(n: &str) -> String {
    let n = n.trim().rsplit(['/', '\\']).next().unwrap_or(n).to_ascii_lowercase();
    n.strip_suffix(".exe").map(str::to_string).unwrap_or(n)
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

/// Environment variable every session's process carries. Its value is `<daemon>:<session id>:<spawn time>` (`<daemon>`
/// is the daemon's pid and its registry instance, `<pid>.<n>`), and
/// children inherit it, so a process that left the session's tree (`setsid`, a double fork, a reparent to init) can
/// still be found by it.
pub const MARK_ENV: &str = "AXELSTRA_MARK";

/// The `AXELSTRA_MARK` entry prefix that names every process of session `id` under `daemon` (see `MARK_ENV`), whatever
/// its spawn time (a woken session keeps its id and gets a new time).
pub fn mark_prefix(daemon: &str, id: u32) -> String {
    format!("{MARK_ENV}={daemon}:{id}:")
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
                out.insert(e.th32ProcessID, clean_name(&String::from_utf16_lossy(&e.szExeFile[..len])));
            }
            ok = Process32NextW(h, &mut e);
        }
        CloseHandle(h);
    }
    #[cfg(unix)]
    if let Ok(o) = std::process::Command::new("ps").args(["-A", "-o", "pid=,comm="]).output() {
        for (pid, name) in parse_ps_names(&String::from_utf8_lossy(&o.stdout)) {
            if pids.contains(&pid) {
                out.insert(pid, clean_name(&name));
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
        let block = b"PATH=/bin\0AXELSTRA_MARK=7.1:12:1700\0HOME=/h\0";
        assert!(environ_has(block, &mark_prefix("7.1", 12)));
        assert!(!environ_has(block, &mark_prefix("7.1", 1)), "id 1 must not match id 12");
        assert!(!environ_has(block, &mark_prefix("8.1", 12)), "another daemon's session");
        assert!(!environ_has(block, &mark_prefix("7.0", 12)), "another registry of the same process");
        assert!(!environ_has(block, &mark_prefix("7", 12)));
        assert!(!environ_has(b"", &mark_prefix("7.1", 12)));
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
    fn parses_ps_tables_of_linux_and_macos() {
        let mac = "    1     0     1  10240
  501     1   501   2048
  junk line
  502   501   501
";
        let t = parse_ps_table(mac);
        assert_eq!(t.len(), 2);
        assert_eq!(t[1], Proc { pid: 501, ppid: 1, pgid: 501, rss_kib: 2048 });
    }

    #[test]
    fn parses_ps_names_with_paths_and_spaces() {
        let out = "  1 /sbin/launchd
  77 /Applications/Google Chrome.app/Contents/MacOS/Google Chrome
 9 bash
 x bad
 10
";
        let v = parse_ps_names(out);
        assert_eq!(v.len(), 3);
        assert_eq!(clean_name(&v[1].1), "google chrome");
        assert_eq!(clean_name(&v[0].1), "launchd");
    }

    #[test]
    fn clean_name_handles_both_separators_and_exe() {
        assert_eq!(clean_name(r"C:\Program Files\Git\bin\Bash.EXE"), "bash");
        assert_eq!(clean_name("/usr/bin/Node"), "node");
        assert_eq!(clean_name("cargo.exe"), "cargo");
        assert_eq!(clean_name("  zsh "), "zsh");
    }

    #[cfg(any(windows, target_os = "macos", target_os = "linux"))]
    #[test]
    fn own_process_reports_memory() {
        let me = std::process::id();
        let s = snapshot();
        let p = s.procs.iter().find(|p| p.pid == me).expect("self in table");
        assert!(p.rss_kib > 0, "rss of own process");
        assert_eq!(names_of(&HashSet::from([me])).len(), 1);
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
