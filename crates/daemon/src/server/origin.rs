//! Who is on the other end of a connection, decided by the operating system and not by a secret the caller presents.
//!
//! An agent runs as the same OS user as the app, so it can read the `token` file and every environment variable it
//! was given. A role derived from "who knows the token" therefore lets a hostile agent promote itself. The daemon
//! owns the agents' process trees (one Job Object per session on Windows, process groups on Unix), so it can ask the
//! kernel whether the process on the other end of a connection belongs to one of them. If it does, the connection
//! is role=agent whatever it sends in `hello`; the token file only authenticates, it never grants the user role.
//! See docs/s3-app-secret.md.

use super::*;

/// The "session" an agent connection is bound to when its process is known to be an agent's (or cannot be examined) but
/// the session it belongs to is not known. Session ids start at 1, so nothing matches it as an inbox or a task file.
pub(super) const UNBOUND: u32 = 0;

/// The process on the other end of a connection: its pid and when that process started. A pid alone can be handed to
/// another process once the client exits; the start time tells the two apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Peer {
    pub pid: u32,
    /// Start time of `pid` read when the connection was accepted; `None` when the OS would not tell.
    pub start: Option<u64>,
}

impl Peer {
    pub(super) fn new(pid: u32) -> Peer {
        Peer { pid, start: process_start(pid) }
    }

    /// True when `pid` still names the process that connected (alive and started at the same moment). A process whose
    /// start time was not readable at accept time cannot be vouched for, so it counts as gone.
    fn still_same(&self) -> bool {
        self.start.is_some() && process_start(self.pid) == self.start
    }
}

impl Registry {
    /// The agent session the client process `peer` belongs to: `Some(id)` for a process inside that session's tree,
    /// `Some(UNBOUND)` for a process that must be treated as an agent without knowing which one, `None` for a
    /// process outside every session (the app, or a CLI run by the user in their own terminal).
    ///
    /// `peer` is `None` only where the OS does not tell the pid of a socket peer: nothing can be said then.
    pub(super) fn origin_of(&self, peer: Option<Peer>) -> Option<u32> {
        let peer = peer?;
        let pid = peer.pid;
        // a client that is already gone (or whose pid now belongs to another process) cannot be examined: it might
        // have said what it wanted and exited, and a connection that could not be classified must not get the
        // user's role
        if !peer.still_same() {
            return Some(UNBOUND);
        }
        let owner = self.sessions.lock_recover().iter().find(|(_, s)| s.owns_pid(pid)).map(|(id, _)| *id);
        owner.or_else(|| orphan_of_ours(pid).then_some(UNBOUND))
    }
}

/// Creation time of process `pid` (any monotone-per-process-lifetime value; only compared for equality), `None` when
/// the process does not exist or cannot be examined.
#[cfg(windows)]
pub(super) fn process_start(pid: u32) -> Option<u64> {
    use windows_sys::Win32::Foundation::{CloseHandle, FILETIME};
    use windows_sys::Win32::System::Threading::{GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
    // the only unsafe of this check: a handle opened, asked once and closed again
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return None;
        }
        let zero = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
        let (mut created, mut e, mut k, mut u) = (zero, zero, zero, zero);
        let ok = GetProcessTimes(h, &mut created, &mut e, &mut k, &mut u);
        CloseHandle(h);
        (ok != 0).then(|| (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime))
    }
}

/// Linux: field 22 of `/proc/<pid>/stat` (start time in clock ticks since boot). Elsewhere `ps -o lstart=`.
#[cfg(unix)]
pub(super) fn process_start(pid: u32) -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        // the command name (field 2) may hold spaces and parentheses: fields restart after the last ')'
        let rest = stat.rsplit_once(')')?.1;
        rest.split_whitespace().nth(19)?.parse().ok()
    }
    #[cfg(not(target_os = "linux"))]
    {
        let out = std::process::Command::new("ps").args(["-o", "lstart=", "-p", &pid.to_string()]).output().ok()?;
        let text = String::from_utf8_lossy(&out.stdout);
        let text = text.trim();
        if !out.status.success() || text.is_empty() {
            return None;
        }
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        text.hash(&mut h);
        Some(h.finish())
    }
}

/// Windows: a Job Object tracks every descendant, even those whose parent died, so there are no orphans to find.
#[cfg(windows)]
fn orphan_of_ours(_pid: u32) -> bool {
    false
}

/// Unix: a process a session started and that was then orphaned is adopted by the daemon (see `adopt_orphans`), so
/// "descends from the daemon" covers what left its session's process group.
#[cfg(unix)]
fn orphan_of_ours(pid: u32) -> bool {
    // the daemon itself is the root of that tree, not an orphan of it (an in-process client is not an agent)
    pid != std::process::id() && axelstra_core::procs::snapshot().descendants(&[std::process::id()]).contains(&pid)
}

/// Makes the daemon the adopter of orphaned descendants (Linux), so a process that double-forks out of its session's
/// tree still hangs below the daemon and `orphan_of_ours` finds it. Other Unixes have no such call: there an agent
/// that detaches itself completely is told apart by its process group only.
pub(crate) fn adopt_orphans() {
    #[cfg(target_os = "linux")]
    unsafe {
        libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0);
    }
}

/// Process id of the client end of a named pipe instance.
#[cfg(windows)]
pub(super) fn pipe_peer(server: &tokio::net::windows::named_pipe::NamedPipeServer) -> Option<Peer> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::System::Pipes::GetNamedPipeClientProcessId;
    let mut pid = 0u32;
    let ok = unsafe { GetNamedPipeClientProcessId(server.as_raw_handle().cast(), &mut pid) };
    (ok != 0 && pid != 0).then(|| Peer::new(pid))
}

/// Process id of the client end of a Unix socket (SO_PEERCRED / LOCAL_PEERPID).
#[cfg(unix)]
pub(super) fn socket_peer(conn: &tokio::net::UnixStream) -> Option<Peer> {
    conn.peer_cred().ok().and_then(|c| c.pid()).and_then(|p| u32::try_from(p).ok()).map(Peer::new)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn own_process_is_the_same_process() {
        let p = Peer::new(std::process::id());
        assert!(p.start.is_some(), "start time of our own process must be readable");
        assert!(p.still_same());
    }

    #[test]
    fn reused_pid_with_another_start_time_is_not_the_same() {
        let mut p = Peer::new(std::process::id());
        p.start = p.start.map(|s| s.wrapping_add(1));
        assert!(!p.still_same(), "same pid, different start time: the pid was reused");
    }

    #[test]
    fn unreadable_start_time_is_not_vouched_for() {
        let p = Peer { pid: std::process::id(), start: None };
        assert!(!p.still_same());
    }

    #[test]
    fn exited_process_is_gone() {
        let mut cmd = if cfg!(windows) {
            let mut c = std::process::Command::new("cmd");
            c.args(["/C", "exit 0"]);
            c
        } else {
            std::process::Command::new("true")
        };
        let mut child = cmd.spawn().unwrap();
        let p = Peer::new(child.id());
        child.wait().unwrap();
        // an open handle keeps the dead process object (and its times) readable on Windows
        drop(child);
        // a handle another test's child inherited can keep the dead object around for a moment
        let end = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while p.still_same() && std::time::Instant::now() < end {
            std::thread::yield_now();
        }
        assert!(!p.still_same());
    }
}
