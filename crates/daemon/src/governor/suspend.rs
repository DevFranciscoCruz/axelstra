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

    /// The two real-process tests share the process-wide `tracked()` set, and `resume_all_tracked`
    /// continues every pid in it: run them one at a time so one never resumes the other's child.
    fn serial() -> std::sync::MutexGuard<'static, ()> {
        static L: std::sync::Mutex<()> = std::sync::Mutex::new(());
        L.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// A child that appends to a heartbeat file in a tight loop. It touches nothing but that file.
    struct Beat {
        c: std::process::Child,
        file: std::path::PathBuf,
    }

    impl Beat {
        fn new(tag: &str) -> Beat {
            let dir = std::env::temp_dir();
            let name = format!("axelstra-beat-{}-{tag}.txt", std::process::id());
            let file = dir.join(&name);
            let _ = std::fs::write(&file, b"");
            let mut c = if cfg!(windows) {
                let mut c = Command::new("cmd");
                c.args(["/C", &format!("for /L %i in (1,1,2000000000) do @echo x>>{name}")]);
                c
            } else {
                let mut c = Command::new("sh");
                c.args(["-c", &format!("while :; do echo x >> {name}; done")]);
                c
            };
            let c = c.current_dir(&dir).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
            Beat { c, file }
        }

        fn pid(&self) -> u32 {
            self.c.id()
        }

        fn size(&self) -> u64 {
            std::fs::metadata(&self.file).map(|m| m.len()).unwrap_or(0)
        }

        /// True once the file has not grown over `quiet` consecutive samples. A suspended child
        /// cannot write; a running one writes thousands of times a second.
        fn frozen(&self, wait: Duration) -> bool {
            let t = Instant::now();
            loop {
                let a = self.size();
                std::thread::sleep(Duration::from_millis(250));
                if self.size() == a && self.size() > 0 {
                    std::thread::sleep(Duration::from_millis(250));
                    if self.size() == a {
                        return true;
                    }
                }
                if t.elapsed() > wait {
                    return false;
                }
            }
        }

        /// True as soon as the file grows.
        fn beating(&self, wait: Duration) -> bool {
            let a = self.size();
            let t = Instant::now();
            while self.size() == a {
                if t.elapsed() > wait {
                    return false;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            true
        }

        fn end(mut self) {
            let _ = self.c.kill();
            let _ = self.c.wait();
            let _ = std::fs::remove_file(&self.file);
        }
    }

    /// One real cycle on a child this test started: its heartbeat stops, then runs again, then it is killed.
    #[test]
    fn a_real_cycle_stops_and_continues_a_child_the_test_owns() {
        let s = OsSuspender;
        if !s.supported() {
            return;
        }
        let _g = serial();
        let b = Beat::new("cycle");
        let pid = b.pid();
        assert!(b.beating(Duration::from_secs(60)), "the child never started writing");
        let stopped = s.suspend(&[pid]);
        assert_eq!(stopped, vec![pid], "could not stop our own child");
        assert!(b.frozen(Duration::from_secs(60)), "the child kept writing after suspend");
        s.resume(&[pid]);
        assert!(b.beating(Duration::from_secs(60)), "the child stayed stopped after resume");
        assert!(!tracked().lock_recover().contains(&pid));
        b.end();
    }

    #[test]
    fn resume_all_tracked_lets_go_of_everything_the_os_suspender_stopped() {
        let s = OsSuspender;
        if !s.supported() {
            return;
        }
        let _g = serial();
        let b = Beat::new("all");
        let pid = b.pid();
        assert!(b.beating(Duration::from_secs(60)), "the child never started writing");
        assert_eq!(s.suspend(&[pid]), vec![pid]);
        assert!(tracked().lock_recover().contains(&pid));
        assert!(b.frozen(Duration::from_secs(60)), "the child kept writing after suspend");
        resume_all_tracked();
        assert!(!tracked().lock_recover().contains(&pid));
        assert!(b.beating(Duration::from_secs(60)), "still stopped");
        b.end();
    }

    #[test]
    fn the_daemon_and_low_pids_are_never_stopped() {
        let s = OsSuspender;
        assert!(s.suspend(&[std::process::id(), 0, 1]).is_empty());
    }
}
