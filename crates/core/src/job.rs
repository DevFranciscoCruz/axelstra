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
            let child = cmd.spawn()?;
            let ok = unsafe { AssignProcessToJobObject(self.0, child.as_raw_handle() as HANDLE) };
            if ok == 0 {
                bail!("AssignProcessToJobObject failed");
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
        /// Private memory right now of every process in the job (what a task manager would add up).
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
                        total += c.PrivateUsage as u64;
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
    use std::os::unix::process::CommandExt;
    use std::sync::Mutex;

    /// Unix has no Job Object: each tracked process leads its own process group
    /// (portable-pty makes the pty child a session leader; `spawn` below uses
    /// `process_group(0)`), and killing the group takes the whole tree with it.
    pub struct Job(Mutex<Vec<i32>>);

    impl Job {
        pub fn new() -> Result<Job> {
            Ok(Job(Default::default()))
        }

        pub fn spawn(&self, cmd: &mut Command) -> Result<Child> {
            cmd.process_group(0);
            let c = cmd.spawn()?;
            self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push(c.id() as i32);
            Ok(c)
        }

        pub fn assign_pid(&self, pid: u32) -> Result<()> {
            self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push(pid as i32);
            Ok(())
        }

        pub fn kill(&self) {
            for pgid in self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner).iter() {
                // the group first (children), the leader itself in case it left the group
                unsafe {
                    libc::killpg(*pgid, libc::SIGKILL);
                    libc::kill(*pgid, libc::SIGKILL);
                }
            }
        }

        /// (resident memory bytes of the groups right now, cpu ticks: not tracked on unix)
        pub fn usage(&self) -> (u64, u64) {
            let groups = self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
            let Ok(o) = Command::new("ps").args(["-A", "-o", "pgid=,rss="]).output() else { return (0, 0) };
            let mut kib = 0u64;
            for l in String::from_utf8_lossy(&o.stdout).lines() {
                let mut f = l.split_whitespace();
                if let (Some(g), Some(r)) = (f.next().and_then(|x| x.parse::<i32>().ok()), f.next().and_then(|x| x.parse::<u64>().ok())) {
                    if groups.contains(&g) {
                        kib += r;
                    }
                }
            }
            (kib * 1024, 0)
        }
    }

    impl Job {
        pub fn memory_now(&self) -> u64 {
            self.usage().0
        }
    }

    // like KILL_ON_JOB_CLOSE: dropping the session ends its processes
    impl Drop for Job {
        fn drop(&mut self) {
            self.kill();
        }
    }
}

pub use imp::Job;
