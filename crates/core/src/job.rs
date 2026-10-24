//! Process-tree ownership. On Windows a Job Object with KILL_ON_JOB_CLOSE
//! guarantees agents die with the daemon and lets us kill a whole tree; on
//! Unix the same is done with process groups.

use anyhow::{bail, Result};
use std::process::{Child, Command};

#[cfg(windows)]
mod imp {
    use super::*;
    use std::os::windows::io::AsRawHandle;
    use std::os::windows::process::CommandExt;
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::System::JobObjects::*;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    pub struct Job(HANDLE);
    unsafe impl Send for Job {}
    // Job Object handles are safe to use from several threads
    unsafe impl Sync for Job {}

    impl Job {
        pub fn new() -> Result<Job> {
            unsafe {
                let h = CreateJobObjectW(std::ptr::null(), std::ptr::null());
                if h.is_null() {
                    bail!("CreateJobObjectW failed");
                }
                let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
                info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                let ok = SetInformationJobObject(
                    h,
                    JobObjectExtendedLimitInformation,
                    &info as *const _ as *const _,
                    std::mem::size_of_val(&info) as u32,
                );
                if ok == 0 {
                    CloseHandle(h);
                    bail!("SetInformationJobObject failed");
                }
                Ok(Job(h))
            }
        }

        pub fn spawn(&self, cmd: &mut Command) -> Result<Child> {
            cmd.creation_flags(CREATE_NO_WINDOW);
            let mut child = cmd.spawn()?;
            let ok = unsafe { AssignProcessToJobObject(self.0, child.as_raw_handle() as HANDLE) };
            if ok == 0 {
                // a child outside the job would outlive its owner and could not be killed as a tree: do not leave it running
                let err = std::io::Error::last_os_error();
                let _ = child.kill();
                let _ = child.wait();
                bail!("AssignProcessToJobObject failed: {err}");
            }
            Ok(child)
        }

        /// For processes spawned elsewhere (portable-pty), assign by pid.
        pub fn assign_pid(&self, pid: u32) -> Result<()> {
            use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE};
            unsafe {
                let h = OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, 0, pid);
                if h.is_null() {
                    bail!("OpenProcess({pid}) failed");
                }
                let ok = AssignProcessToJobObject(self.0, h);
                CloseHandle(h);
                if ok == 0 {
                    bail!("AssignProcessToJobObject({pid}) failed");
                }
            }
            Ok(())
        }

        pub fn kill(&self) {
            unsafe { TerminateJobObject(self.0, 1) };
        }

        /// Nothing to forget: a Job Object tracks its processes itself.
        pub fn forget(&self, _pid: u32) {}

        /// Unix needs the leader pid to clean up stragglers; here the whole job is killed.
        pub fn kill_and_forget(&self, _pid: u32) {
            self.kill();
        }

        pub fn usage_in(&self, _snap: &crate::procs::Snapshot) -> (u64, u64) {
            self.usage()
        }

        pub fn memory_now_in(&self, _snap: &crate::procs::Snapshot) -> u64 {
            self.memory_now()
        }

        /// (peak job memory bytes, total user+kernel CPU in 100ns ticks)
        pub fn usage(&self) -> (u64, u64) {
            unsafe {
                let mut ext: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
                QueryInformationJobObject(
                    self.0,
                    JobObjectExtendedLimitInformation,
                    &mut ext as *mut _ as *mut _,
                    std::mem::size_of_val(&ext) as u32,
                    std::ptr::null_mut(),
                );
                let mut acc: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION = std::mem::zeroed();
                QueryInformationJobObject(
                    self.0,
                    JobObjectBasicAccountingInformation,
                    &mut acc as *mut _ as *mut _,
                    std::mem::size_of_val(&acc) as u32,
                    std::ptr::null_mut(),
                );
                (ext.PeakJobMemoryUsed as u64, (acc.TotalUserTime + acc.TotalKernelTime) as u64)
            }
        }
    }

    impl Job {
        /// RAM in use right now by every process in the job (working set). Not the commit charge (PrivateUsage): that one
        /// counts memory a process only reserved and can be twice the real RAM for node and claude.
        pub fn memory_now(&self) -> u64 {
            use windows_sys::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX};
            use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
            unsafe {
                // JOBOBJECT_BASIC_PROCESS_ID_LIST: two counts, then the ids (room for 1000 here)
                let mut buf = vec![0u64; 2 + 1000];
                let ok = QueryInformationJobObject(
                    self.0,
                    JobObjectBasicProcessIdList,
                    buf.as_mut_ptr() as *mut _,
                    (buf.len() * 8) as u32,
                    std::ptr::null_mut(),
                );
                if ok == 0 {
                    return 0;
                }
                let n = (buf[0] >> 32) as usize; // NumberOfProcessIdsInList follows NumberOfAssignedProcesses (u32 each)
                let ids = (buf.as_ptr() as *const u8).add(8) as *const usize;
                let mut total = 0u64;
                for i in 0..n.min(1000) {
                    let pid = *ids.add(i) as u32;
                    let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
                    if h.is_null() {
                        continue;
                    }
                    let mut c: PROCESS_MEMORY_COUNTERS_EX = std::mem::zeroed();
                    c.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32;
                    if GetProcessMemoryInfo(h, &mut c as *mut _ as *mut PROCESS_MEMORY_COUNTERS, c.cb) != 0 {
                        total += c.WorkingSetSize as u64;
                    }
                    CloseHandle(h);
                }
                total
            }
        }
    }

    impl Drop for Job {
        fn drop(&mut self) {
            unsafe { CloseHandle(self.0) };
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::*;
    use crate::procs::{self, Snapshot};
    use crate::sync::LockRecover;
    use std::os::unix::process::CommandExt;
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    /// Unix has no Job Object: each tracked process leads its own process group
    /// (portable-pty makes the pty child a session leader; `spawn` below uses
    /// `process_group(0)`), and killing the group takes the whole tree with it.
    ///
    /// A pgid is only ever signalled while its leader is still our child, reaped or not: a zombie leader keeps the
    /// pid (and so the group id) reserved, but once somebody `wait()`ed for it the number may be handed to an
    /// unrelated process. `kill` therefore drops every pid that is no longer our child instead of signalling it.
    pub struct Job(Mutex<Vec<i32>>);

    /// True while `pid` is a child of ours that has not been reaped yet (running or zombie). Looks without reaping.
    fn is_unreaped_child(pid: i32) -> bool {
        unsafe {
            let mut si: libc::siginfo_t = std::mem::zeroed();
            let r = libc::waitid(libc::P_PID, pid as libc::id_t, &mut si, libc::WEXITED | libc::WNOHANG | libc::WNOWAIT);
            // ECHILD: reaped already (or never ours)
            !(r == -1 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ECHILD))
        }
    }

    /// Reaps `pid` if it already exited; true when it is gone for good.
    fn reap(pid: i32) -> bool {
        let mut st = 0;
        let r = unsafe { libc::waitpid(pid, &mut st, libc::WNOHANG) };
        r == pid || (r == -1 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ECHILD))
    }

    impl Job {
        pub fn new() -> Result<Job> {
            Ok(Job(Default::default()))
        }

        pub fn spawn(&self, cmd: &mut Command) -> Result<Child> {
            cmd.process_group(0);
            let c = cmd.spawn()?;
            self.0.lock_recover().push(c.id() as i32);
            Ok(c)
        }

        pub fn assign_pid(&self, pid: u32) -> Result<()> {
            self.0.lock_recover().push(pid as i32);
            Ok(())
        }

        /// Stops tracking `pid` (call after reaping it, so its group id is never signalled once it can be reused).
        pub fn forget(&self, pid: u32) {
            self.0.lock_recover().retain(|p| *p != pid as i32);
        }

        /// Kills the group of `pid` right now, whether or not it was reaped, then stops tracking it. For a caller that
        /// just reaped the leader and wants to clean up stragglers (nothing can have reused the number yet).
        pub fn kill_and_forget(&self, pid: u32) {
            unsafe {
                libc::killpg(pid as i32, libc::SIGKILL);
            }
            self.forget(pid);
        }

        pub fn kill(&self) {
            let mut g = self.0.lock_recover();
            g.retain(|pgid| is_unreaped_child(*pgid));
            for pgid in g.iter() {
                // the group first (children), the leader itself in case it left the group
                unsafe {
                    libc::killpg(*pgid, libc::SIGKILL);
                    libc::kill(*pgid, libc::SIGKILL);
                }
            }
        }

        /// (resident memory bytes of the groups right now, cpu ticks: not tracked on unix)
        pub fn usage(&self) -> (u64, u64) {
            self.usage_in(&procs::snapshot_cached(Duration::from_secs(1)))
        }

        /// Same, on a process table the caller already took (one `ps` per poll for any number of jobs).
        pub fn usage_in(&self, snap: &Snapshot) -> (u64, u64) {
            let groups = self.0.lock_recover().clone();
            (snap.rss_of_groups(&groups) * 1024, 0)
        }
    }

    impl Job {
        pub fn memory_now(&self) -> u64 {
            self.usage().0
        }

        pub fn memory_now_in(&self, snap: &Snapshot) -> u64 {
            self.usage_in(snap).0
        }
    }

    // like KILL_ON_JOB_CLOSE: dropping the session ends its processes, and the dead ones are reaped (no zombies)
    impl Drop for Job {
        fn drop(&mut self) {
            self.kill();
            let mut left: Vec<i32> = std::mem::take(&mut *self.0.lock_recover());
            let end = Instant::now() + Duration::from_millis(500);
            while !left.is_empty() && Instant::now() < end {
                left.retain(|p| !reap(*p));
                if !left.is_empty() {
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn alive(pid: i32) -> bool {
            unsafe { libc::kill(pid, 0) == 0 }
        }

        fn sleeper(job: &Job) -> Child {
            job.spawn(Command::new("sh").args(["-c", "sleep 60 & wait"])).unwrap()
        }

        #[test]
        fn kill_takes_the_group_and_drop_reaps_the_leader() {
            let job = Job::new().unwrap();
            let c = sleeper(&job);
            let pid = c.id() as i32;
            std::thread::sleep(Duration::from_millis(200));
            job.kill();
            drop(job);
            // reaped by Drop: not a zombie any more, so the pid is gone
            let end = Instant::now() + Duration::from_secs(3);
            while alive(pid) && Instant::now() < end {
                std::thread::sleep(Duration::from_millis(20));
            }
            assert!(!alive(pid), "leader {pid} left as zombie or alive");
            std::mem::forget(c);
        }

        #[test]
        fn a_reaped_child_is_not_signalled_again() {
            let job = Job::new().unwrap();
            let mut c = job.spawn(Command::new("sh").args(["-c", "exit 0"])).unwrap();
            c.wait().unwrap();
            let pid = c.id() as i32;
            assert!(!is_unreaped_child(pid));
            job.kill();
            assert!(job.0.lock_recover().is_empty(), "reaped pid must be forgotten, not killed");
        }

        #[test]
        fn zombie_leader_still_counts_as_ours_until_reaped() {
            let job = Job::new().unwrap();
            let mut c = job.spawn(Command::new("sh").args(["-c", "exit 0"])).unwrap();
            std::thread::sleep(Duration::from_millis(300));
            assert!(is_unreaped_child(c.id() as i32));
            c.wait().unwrap();
            assert!(!is_unreaped_child(c.id() as i32));
        }

        #[test]
        fn kill_and_forget_clears_stragglers_of_a_reaped_leader() {
            let job = Job::new().unwrap();
            // the leader exits at once and leaves a background sleeper in its group
            let mut c = job.spawn(Command::new("sh").args(["-c", "sleep 60 & echo $!"]).stdout(std::process::Stdio::piped())).unwrap();
            let mut out = String::new();
            std::io::Read::read_to_string(c.stdout.as_mut().unwrap(), &mut out).unwrap();
            let kid: i32 = out.trim().parse().unwrap();
            c.wait().unwrap();
            assert!(alive(kid));
            job.kill_and_forget(c.id());
            let end = Instant::now() + Duration::from_secs(3);
            while alive(kid) && Instant::now() < end {
                std::thread::sleep(Duration::from_millis(20));
            }
            assert!(!alive(kid), "straggler {kid} survived kill_and_forget");
        }
    }
}

pub use imp::Job;
