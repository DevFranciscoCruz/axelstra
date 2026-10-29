//! Stopping and continuing processes: the only OS code of the pause feature, behind the `Suspender` trait so tests
//! inject a fake and never touch a real process they do not own.
//!
//! Unix: `SIGSTOP` / `SIGCONT` to each pid of a process tree (not to the process group: the command of `axelstra lock`
//! shares its group with the lock process and the agent's shell, which must keep running). Windows:
//! `NtSuspendProcess` / `NtResumeProcess` (ntdll) per process of the tree. `DebugActiveProcess` is not used: it attaches
//! a debugger and kills the target when the daemon dies.
//!
//! A suspended process is frozen for good if nobody resumes it, so every pid this module stops is also recorded in a
//! process-wide set: `resume_all_tracked` (the panic hook and the shutdown path) lets all of them go.

use axelstra_core::sync::LockRecover;
use std::collections::HashSet;
use std::sync::Mutex;

/// Stops and continues processes. Both are best effort per pid (a process that already ended is not an error).
pub trait Suspender: Send + Sync {
    /// Whether this OS can suspend at all (tests of the real cycle skip when not).
    fn supported(&self) -> bool {
        true
    }
    /// Stops each pid. Returns the pids that were really stopped.
    fn suspend(&self, pids: &[u32]) -> Vec<u32>;
    /// Continues each pid.
    fn resume(&self, pids: &[u32]);
}

/// Every pid `OsSuspender` stopped and has not continued yet.
fn tracked() -> &'static Mutex<HashSet<u32>> {
    static T: std::sync::OnceLock<Mutex<HashSet<u32>>> = std::sync::OnceLock::new();
    T.get_or_init(|| Mutex::new(HashSet::new()))
}

/// Continues every process this module stopped. Safe from a panic hook: it takes only a leaf lock and no allocation
/// that depends on the daemon's state.
pub fn resume_all_tracked() {
    let pids: Vec<u32> = tracked().lock_recover().drain().collect();
    for pid in pids {
        os::resume_one(pid);
    }
}

/// The real thing.
pub struct OsSuspender;

impl Suspender for OsSuspender {
    fn suspend(&self, pids: &[u32]) -> Vec<u32> {
        let me = std::process::id();
        let mut done = Vec::new();
        for &pid in pids {
            // never the daemon itself, never pid 0 / 1 (a signal to 0 would reach our own group)
            if pid == me || pid <= 1 {
                continue;
            }
            // recorded first: a crash between the stop and the record must not leave it unlisted
            let fresh = tracked().lock_recover().insert(pid);
            if os::suspend_one(pid) {
                done.push(pid);
            } else if fresh {
                tracked().lock_recover().remove(&pid);
            }
        }
        done
    }

    fn resume(&self, pids: &[u32]) {
        for &pid in pids {
            tracked().lock_recover().remove(&pid);
            os::resume_one(pid);
        }
    }
}

#[cfg(unix)]
mod os {
    pub fn suspend_one(pid: u32) -> bool {
        signal(pid, libc::SIGSTOP)
    }

    pub fn resume_one(pid: u32) {
        signal(pid, libc::SIGCONT);
    }

    fn signal(pid: u32, sig: libc::c_int) -> bool {
        let Ok(p) = libc::pid_t::try_from(pid) else { return false };
        // SAFETY: `kill` with a positive pid signals exactly that process; pids <= 1 were excluded by the caller, so the
        // call can never address a process group (0, negative) or every process (-1).
        p > 1 && unsafe { libc::kill(p, sig) } == 0
    }
}

#[cfg(windows)]
mod os {
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_SUSPEND_RESUME};

    #[link(name = "ntdll")]
    extern "system" {
        fn NtSuspendProcess(process: HANDLE) -> i32;
        fn NtResumeProcess(process: HANDLE) -> i32;
    }

    fn with_handle(pid: u32, f: unsafe extern "system" fn(HANDLE) -> i32) -> bool {
        // SAFETY: a handle is opened for one call and closed again; `f` is one of the two ntdll functions above, which only
        // read the handle. NTSTATUS >= 0 is success.
        unsafe {
            let h = OpenProcess(PROCESS_SUSPEND_RESUME, 0, pid);
            if h.is_null() {
                return false;
            }
            let status = f(h);
            CloseHandle(h);
            status >= 0
        }
    }

    pub fn suspend_one(pid: u32) -> bool {
        with_handle(pid, NtSuspendProcess)
    }

    pub fn resume_one(pid: u32) {
        with_handle(pid, NtResumeProcess);
    }
}

#[cfg(not(any(unix, windows)))]
mod os {
    pub fn suspend_one(_pid: u32) -> bool {
        false
    }
    pub fn resume_one(_pid: u32) {}
}

#[cfg(test)]
pub mod fake {
    use super::*;

    /// Records what was stopped; nothing real is touched.
    #[derive(Default)]
    pub struct FakeSuspender {
        pub stopped: Mutex<HashSet<u32>>,
        pub log: Mutex<Vec<String>>,
    }

    impl Suspender for FakeSuspender {
        fn suspend(&self, pids: &[u32]) -> Vec<u32> {
            let mut s = self.stopped.lock_recover();
            for p in pids {
                s.insert(*p);
            }
            self.log.lock_recover().push(format!("suspend {pids:?}"));
            pids.to_vec()
        }

        fn resume(&self, pids: &[u32]) {
            let mut s = self.stopped.lock_recover();
            for p in pids {
                s.remove(p);
            }
            self.log.lock_recover().push(format!("resume {pids:?}"));
        }
    }

    impl FakeSuspender {
        pub fn is_stopped(&self, pid: u32) -> bool {
            self.stopped.lock_recover().contains(&pid)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    fn child() -> std::process::Child {
        let mut c = if cfg!(windows) {
            let mut c = Command::new("ping");
            c.args(["-n", "60", "127.0.0.1"]);
            c
        } else {
            let mut c = Command::new("sleep");
            c.arg("60");
            c
        };
        c.stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap()
    }

    /// Whether the process is stopped, by what the OS says.
    #[cfg(target_os = "linux")]
    fn is_stopped(pid: u32) -> bool {
        std::fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|s| s.rsplit_once(')').is_some_and(|r| r.1.trim_start().starts_with('T')))
    }

    #[cfg(all(unix, not(target_os = "linux")))]
    fn is_stopped(pid: u32) -> bool {
        let o = Command::new("ps").args(["-o", "stat=", "-p", &pid.to_string()]).output().unwrap();
        String::from_utf8_lossy(&o.stdout).trim().starts_with('T')
    }

    /// Windows: a suspended process has every thread waiting with reason Suspended (read through PowerShell).
    #[cfg(windows)]
    fn is_stopped(pid: u32) -> bool {
        let script = format!(
            "(Get-Process -Id {pid}).Threads | Where-Object {{ $_.ThreadState -ne 'Wait' -or $_.WaitReason -ne 'Suspended' }} | Measure-Object | ForEach-Object Count"
        );
        let o = Command::new("powershell").args(["-NoProfile", "-Command", &script]).output().unwrap();
        String::from_utf8_lossy(&o.stdout).trim() == "0"
    }

    /// One real cycle on a child this test started: stopped, then running again, then killed.
    #[test]
    fn a_real_cycle_stops_and_continues_a_child_the_test_owns() {
        let s = OsSuspender;
        if !s.supported() {
            return;
        }
        let mut c = child();
        let pid = c.id();
        let stopped = s.suspend(&[pid]);
        assert_eq!(stopped, vec![pid], "could not stop our own child");
        let t = Instant::now();
        while !is_stopped(pid) {
            assert!(t.elapsed() < Duration::from_secs(10), "the child never showed as stopped");
            std::thread::sleep(Duration::from_millis(50));
        }
        s.resume(&[pid]);
        let t = Instant::now();
        while is_stopped(pid) {
            assert!(t.elapsed() < Duration::from_secs(10), "the child stayed stopped after resume");
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(tracked().lock_recover().is_empty() || !tracked().lock_recover().contains(&pid));
        let _ = c.kill();
        let _ = c.wait();
    }

    #[test]
    fn resume_all_tracked_lets_go_of_everything_the_os_suspender_stopped() {
        let s = OsSuspender;
        let mut c = child();
        let pid = c.id();
        assert_eq!(s.suspend(&[pid]), vec![pid]);
        assert!(tracked().lock_recover().contains(&pid));
        resume_all_tracked();
        assert!(!tracked().lock_recover().contains(&pid));
        let t = Instant::now();
        while is_stopped(pid) {
            assert!(t.elapsed() < Duration::from_secs(10), "still stopped");
            std::thread::sleep(Duration::from_millis(50));
        }
        let _ = c.kill();
        let _ = c.wait();
    }

    #[test]
    fn the_daemon_and_low_pids_are_never_stopped() {
        let s = OsSuspender;
        assert!(s.suspend(&[std::process::id(), 0, 1]).is_empty());
    }
}
