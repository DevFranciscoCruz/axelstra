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
