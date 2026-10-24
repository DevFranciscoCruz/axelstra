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

impl Registry {
    /// The agent session the client process `peer` belongs to: `Some(id)` for a process inside that session's tree,
    /// `Some(UNBOUND)` for a process that must be treated as an agent without knowing which one, `None` for a
    /// process outside every session (the app, or a CLI run by the user in their own terminal).
    ///
    /// `peer` is `None` only where the OS does not tell the pid of a socket peer: nothing can be said then.
    pub(super) fn origin_of(&self, peer: Option<u32>) -> Option<u32> {
        let pid = peer?;
        // a client that is already gone cannot be examined: it might have said what it wanted and exited, and a
        // connection that could not be classified must not get the user's role
        if !process_exists(pid) {
            return Some(UNBOUND);
        }
        let owner = self.sessions.lock_recover().iter().find(|(_, s)| s.owns_pid(pid)).map(|(id, _)| *id);
        owner.or_else(|| orphan_of_ours(pid).then_some(UNBOUND))
    }
}

#[cfg(windows)]
fn process_exists(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return false;
        }
        CloseHandle(h);
        true
    }
}

#[cfg(unix)]
fn process_exists(pid: u32) -> bool {
    // EPERM still means it exists
    unsafe { libc::kill(pid as i32, 0) == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM) }
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
pub(super) fn pipe_peer(server: &tokio::net::windows::named_pipe::NamedPipeServer) -> Option<u32> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::System::Pipes::GetNamedPipeClientProcessId;
    let mut pid = 0u32;
    let ok = unsafe { GetNamedPipeClientProcessId(server.as_raw_handle().cast(), &mut pid) };
    (ok != 0 && pid != 0).then_some(pid)
}

/// Process id of the client end of a Unix socket (SO_PEERCRED / LOCAL_PEERPID).
#[cfg(unix)]
pub(super) fn socket_peer(conn: &tokio::net::UnixStream) -> Option<u32> {
    conn.peer_cred().ok().and_then(|c| c.pid()).and_then(|p| u32::try_from(p).ok())
}
