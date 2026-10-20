//! Process-tree ownership. On Windows a Job Object with KILL_ON_JOB_CLOSE
//! guarantees agents die with the daemon and lets us kill a whole tree.

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
                (
                    ext.PeakJobMemoryUsed as u64,
                    (acc.TotalUserTime + acc.TotalKernelTime) as u64,
                )
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
    pub struct Job(std::sync::Mutex<Option<u32>>);
    impl Job {
        pub fn new() -> Result<Job> {
            Ok(Job(Default::default()))
        }
        pub fn spawn(&self, cmd: &mut Command) -> Result<Child> {
            let c = cmd.spawn()?;
            *self.0.lock().unwrap() = Some(c.id());
            Ok(c)
        }
        pub fn assign_pid(&self, pid: u32) -> Result<()> {
            *self.0.lock().unwrap() = Some(pid);
            Ok(())
        }
        pub fn kill(&self) {
            if let Some(pid) = *self.0.lock().unwrap() {
                let _ = Command::new("kill").args(["-9", &pid.to_string()]).status();
            }
        }
        pub fn usage(&self) -> (u64, u64) {
            (0, 0)
        }
    }
}

pub use imp::Job;
