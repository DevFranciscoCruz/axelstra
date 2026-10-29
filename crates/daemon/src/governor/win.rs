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
