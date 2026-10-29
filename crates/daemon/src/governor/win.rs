//! Windows memory source: the kernel's memory resource notifications (low and high memory) plus a stop event, waited on
//! together with `WaitForMultipleObjects`. All the `unsafe` of the governor on Windows is in this file.
//!
//! The notification objects stay signaled for as long as the condition holds, so waiting on the same one twice would
//! return at once, forever. The source therefore waits on ONE of them at a time: the "low" object while memory is not
//! low, then the "high" object (memory is plentiful again) once low was seen, and back. Each wait returns once per
//! transition, not once per look.

use super::{MemInfo, MemSource, Stop, Wake};
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows_sys::Win32::System::Memory::{
    CreateMemoryResourceNotification, HighMemoryResourceNotification, LowMemoryResourceNotification, QueryMemoryResourceNotification,
};
use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
use windows_sys::Win32::System::Threading::{CreateEventW, SetEvent, WaitForMultipleObjects};

/// An owned kernel handle, closed on drop.
struct Handle(HANDLE);

// SAFETY: a kernel handle is a process-wide token; the calls we make on it (wait, query, set) are thread-safe.
unsafe impl Send for Handle {}
// SAFETY: see above; no method takes `&mut`.
unsafe impl Sync for Handle {}

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: the handle is valid (checked non-null at creation) and owned only by this struct.
        unsafe { CloseHandle(self.0) };
    }
}

fn checked(h: HANDLE) -> io::Result<Handle> {
    if h.is_null() {
        Err(io::Error::last_os_error())
    } else {
        Ok(Handle(h))
    }
}

pub struct WinSource {
    low: Handle,
    high: Handle,
    /// set by `wake`, manual reset: a stop is final
    stop_ev: Handle,
    /// which notification the next wait uses
    armed_low: AtomicBool,
}

impl WinSource {
    pub fn new() -> io::Result<WinSource> {
        // SAFETY: plain creation calls; the results are checked for null before use.
        let (low, high, stop_ev) = unsafe {
            (
                checked(CreateMemoryResourceNotification(LowMemoryResourceNotification))?,
                checked(CreateMemoryResourceNotification(HighMemoryResourceNotification))?,
                checked(CreateEventW(std::ptr::null(), 1, 0, std::ptr::null()))?,
            )
        };
        let s = WinSource { armed_low: AtomicBool::new(true), low, high, stop_ev };
        // already low: the "low" object is signaled now, so wait for "high" first
        if let Some(true) = s.signaled(&s.low) {
            s.armed_low.store(false, Ordering::SeqCst);
        }
        Ok(s)
    }

    /// Whether a notification object is signaled now.
    fn signaled(&self, h: &Handle) -> Option<bool> {
        let mut state = 0i32;
        // SAFETY: `h` is a live notification handle and `state` is a valid out pointer for the call.
        let ok = unsafe { QueryMemoryResourceNotification(h.0, &mut state) };
        (ok != 0).then_some(state != 0)
    }
}

impl MemSource for WinSource {
    fn snapshot(&self) -> Option<MemInfo> {
        global_status()
    }

    fn wait_change(&self, stop: &Stop, timeout: Duration) -> Wake {
        if stop.is_set() {
            return Wake::Stop;
        }
        let armed_low = self.armed_low.load(Ordering::SeqCst);
        let handles = [if armed_low { self.low.0 } else { self.high.0 }, self.stop_ev.0];
        let ms = timeout.as_millis().min(u32::MAX as u128 - 1) as u32;
        // SAFETY: `handles` holds two live handles owned by `self`, and the count passed is 2.
        let r = unsafe { WaitForMultipleObjects(2, handles.as_ptr(), 0, ms) };
        match r {
            x if x == WAIT_OBJECT_0 => {
                self.armed_low.store(!armed_low, Ordering::SeqCst);
                Wake::Event
            }
            x if x == WAIT_OBJECT_0 + 1 => Wake::Stop,
            x if x == WAIT_TIMEOUT => Wake::Timeout,
            // a failed wait must not spin: sleep the deadline out on the stop token
            _ => {
                if stop.sleep(timeout) {
                    Wake::Stop
                } else {
                    Wake::Timeout
                }
            }
        }
    }

    fn wake(&self) {
        // SAFETY: `stop_ev` is a live event handle owned by `self`.
        unsafe { SetEvent(self.stop_ev.0) };
    }

    fn has_events(&self) -> bool {
        true
    }

    fn name(&self) -> &'static str {
        "windows-memory-notification"
    }
}

/// Physical memory available and the commit charge, from `GlobalMemoryStatusEx`.
pub fn global_status() -> Option<MemInfo> {
    // SAFETY: MEMORYSTATUSEX is plain data; zeroed is valid, and `dwLength` is set as the API requires.
    let mut m: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
    m.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
    // SAFETY: `m` is a valid, correctly sized MEMORYSTATUSEX.
    if unsafe { GlobalMemoryStatusEx(&mut m) } == 0 {
        return None;
    }
    let mb = |b: u64| b / (1024 * 1024);
    let commit_pct =
        (m.ullTotalPageFile > 0).then(|| (m.ullTotalPageFile.saturating_sub(m.ullAvailPageFile) * 100 / m.ullTotalPageFile).min(100) as u8);
    Some(MemInfo { total_mb: mb(m.ullTotalPhys), free_mb: mb(m.ullAvailPhys), commit_pct })
}

// ---- cause: kernel pools, machine counters, per-process stats (direct APIs only, never PDH) ----

use super::cause::{CpuTimes, KernelMem, SysCounters};
use windows_sys::Win32::System::ProcessStatus::{
    GetPerformanceInfo, GetProcessMemoryInfo, PERFORMANCE_INFORMATION, PROCESS_MEMORY_COUNTERS,
};
use windows_sys::Win32::System::Threading::{
    GetProcessHandleCount, GetProcessTimes, GetSystemTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
};

#[link(name = "ntdll")]
extern "system" {
    fn NtQuerySystemInformation(class: u32, info: *mut std::ffi::c_void, len: u32, ret: *mut u32) -> i32;
}

const MB: u64 = 1024 * 1024;

fn perf_info() -> Option<PERFORMANCE_INFORMATION> {
    // SAFETY: plain data, zeroed is valid; `cb` is set as the API requires and the pointer is to our own struct.
    let mut p: PERFORMANCE_INFORMATION = unsafe { std::mem::zeroed() };
    p.cb = std::mem::size_of::<PERFORMANCE_INFORMATION>() as u32;
    (unsafe { GetPerformanceInfo(&mut p, p.cb) } != 0).then_some(p)
}

/// Kernel paged / nonpaged pools and the system cache (which includes the standby list), in MB.
pub fn kernel_and_cache() -> Option<(KernelMem, u64)> {
    let p = perf_info()?;
    let page = p.PageSize as u64;
    let (paged, nonpaged) = (p.KernelPaged as u64 * page / MB, p.KernelNonpaged as u64 * page / MB);
    Some((KernelMem { paged_mb: paged, nonpaged_mb: nonpaged, alert_mb: paged + nonpaged }, p.SystemCache as u64 * page / MB))
}

/// Handles open on the whole system.
pub fn system_handles() -> Option<u64> {
    perf_info().map(|p| p.HandleCount as u64)
}

fn query_sys<const N: usize>(class: u32) -> Option<[u8; N]> {
    let mut buf = [0u8; N];
    let mut ret = 0u32;
    // SAFETY: `buf` is a writable buffer of N bytes and the length passed is N; the call only writes inside it.
    let st = unsafe { NtQuerySystemInformation(class, buf.as_mut_ptr().cast(), N as u32, &mut ret) };
    (st >= 0).then_some(buf)
}

fn u32_at(b: &[u8], off: usize) -> u64 {
    u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]]) as u64
}

fn usize_at(b: &[u8], off: usize) -> u64 {
    let mut a = [0u8; 8];
    a.copy_from_slice(&b[off..off + 8]);
    u64::from_le_bytes(a)
}

/// CPU times from `GetSystemTimes` (kernel includes idle), paging from `SystemPerformanceInformation` (class 2: page
/// reads at byte 80, dirty and mapped page writes at 96 and 104 on 64-bit), commit from `GetPerformanceInfo`, the modified
/// list from `SystemMemoryListInformation` (class 80). Each part is `None` when its call fails.
pub fn system_counters() -> Option<SysCounters> {
    let ft = |f: &windows_sys::Win32::Foundation::FILETIME| ((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64;
    // SAFETY: FILETIME is plain data; the three out pointers are to our own locals.
    let cpu = unsafe {
        let (mut i, mut k, mut u): (windows_sys::Win32::Foundation::FILETIME, _, _) =
            (std::mem::zeroed(), std::mem::zeroed(), std::mem::zeroed());
        (GetSystemTimes(&mut i, &mut k, &mut u) != 0).then(|| {
            let (idle, kern, user) = (ft(&i), ft(&k), ft(&u));
            CpuTimes { idle, kernel: kern.saturating_sub(idle), user }
        })
    };
    let pages = query_sys::<512>(2).map(|b| u32_at(&b, 80) + u32_at(&b, 96) + u32_at(&b, 104));
    let perf = perf_info();
    let page = perf.as_ref().map(|p| p.PageSize as u64).unwrap_or(4096);
    // ZeroPageCount, FreePageCount, ModifiedPageCount, ... (pointer-sized)
    let modified = query_sys::<128>(80).map(|b| usize_at(&b, 16) * page / MB);
    Some(SysCounters {
        cpu,
        pages,
        commit_mb: perf.as_ref().map(|p| p.CommitTotal as u64 * page / MB),
        commit_limit_mb: perf.as_ref().map(|p| p.CommitLimit as u64 * page / MB),
        modified_mb: modified,
    })
}

#[derive(Default)]
pub struct ProcStats {
    pub rss_kib: u64,
    pub handles: Option<u32>,
    pub cpu_100ns: Option<u64>,
    pub io_bytes: Option<u64>,
}

/// Working set, handle count, CPU time and I/O bytes of one process. Zeros / `None` when the process cannot be opened
/// (protected processes, gone since the table was taken).
pub fn process_stats(pid: u32) -> ProcStats {
    let mut st = ProcStats::default();
    if pid == 0 {
        return st;
    }
    // SAFETY: the handle is checked for null and closed below; every out pointer is to a local of the type the API expects.
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return st;
        }
        let mut mc: PROCESS_MEMORY_COUNTERS = std::mem::zeroed();
        mc.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
        if GetProcessMemoryInfo(h, &mut mc, mc.cb) != 0 {
            st.rss_kib = mc.WorkingSetSize as u64 / 1024;
        }
        let mut n = 0u32;
        if GetProcessHandleCount(h, &mut n) != 0 {
            st.handles = Some(n);
        }
        let z = || std::mem::zeroed::<windows_sys::Win32::Foundation::FILETIME>();
        let (mut c, mut e, mut k, mut u) = (z(), z(), z(), z());
        if GetProcessTimes(h, &mut c, &mut e, &mut k, &mut u) != 0 {
            let ft = |f: &windows_sys::Win32::Foundation::FILETIME| ((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64;
            st.cpu_100ns = Some(ft(&k) + ft(&u));
        }
        let mut io: windows_sys::Win32::System::Threading::IO_COUNTERS = std::mem::zeroed();
        if windows_sys::Win32::System::Threading::GetProcessIoCounters(h, &mut io) != 0 {
            st.io_bytes = Some(io.ReadTransferCount + io.WriteTransferCount);
        }
        CloseHandle(h);
    }
    st
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::time::Instant;

    #[test]
    fn reads_the_machine() {
        let i = global_status().unwrap();
        assert!(i.total_mb > 0 && i.free_mb > 0 && i.free_mb <= i.total_mb, "{i:?}");
        assert!(i.commit_pct.is_some());
    }

    #[test]
    fn a_wait_ends_by_deadline() {
        let (s, stop) = (WinSource::new().unwrap(), Stop::default());
        let t = Instant::now();
        let w = s.wait_change(&stop, Duration::from_millis(100));
        assert!(matches!(w, Wake::Timeout | Wake::Event), "{w:?}");
        assert!(t.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn stop_and_wake_end_a_long_wait() {
        let s = Arc::new(WinSource::new().unwrap());
        let stop = Arc::new(Stop::default());
        // the OS may already be signaled low or high: take any such first event out of the way
        let (s2, stop2) = (s.clone(), stop.clone());
        let h = std::thread::spawn(move || loop {
            match s2.wait_change(&stop2, Duration::from_secs(60)) {
                Wake::Event => continue,
                w => return w,
            }
        });
        std::thread::sleep(Duration::from_millis(100));
        let t = Instant::now();
        stop.trigger();
        s.wake();
        assert_eq!(h.join().unwrap(), Wake::Stop);
        assert!(t.elapsed() < Duration::from_secs(5));
    }
}
