//! Process tree lookups: which processes descend from a session's shell.

use std::collections::{HashMap, HashSet};

/// (pid, parent pid) of every process on the machine.
#[cfg(windows)]
fn snapshot() -> Vec<(u32, u32)> {
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
            out.push((e.th32ProcessID, e.th32ParentProcessID));
            ok = Process32NextW(h, &mut e);
        }
        CloseHandle(h);
    }
    out
}

#[cfg(unix)]
fn snapshot() -> Vec<(u32, u32)> {
    let Ok(o) = std::process::Command::new("ps").args(["-A", "-o", "pid=,ppid="]).output() else { return Vec::new() };
    String::from_utf8_lossy(&o.stdout)
        .lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            Some((it.next()?.parse().ok()?, it.next()?.parse().ok()?))
        })
        .collect()
}

/// `roots` and every process below them.
pub fn with_descendants(roots: &[u32]) -> HashSet<u32> {
    descend(&snapshot(), roots)
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
        assert!(snapshot().iter().any(|&(p, _)| p == std::process::id()));
    }
}
