//! Linux memory source: a PSI trigger on `/proc/pressure/memory` (the kernel raises POLLPRI when tasks stalled on memory
//! for a given time in a window), polled together with a socket pair used as the stop wake. The only `unsafe` is the
//! `poll(2)` call.
//!
//! PSI fires on real stalls, which is later than our free-memory lines; the governor's computed deadline covers the
//! stretch before that (see the module docs of `governor`). Needs kernel 4.20+ with PSI on; for an unprivileged writer
//! the kernel (6.x) may refuse the trigger: `new` then fails and the caller samples instead.

use super::{MemInfo, MemSource, Stop, Wake};
use std::fs::OpenOptions;
use std::io::{self, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::net::UnixStream;
use std::time::Duration;

/// "some" stall of 150 ms inside a 1 s window raises the event.
const TRIGGER: &str = "some 150000 1000000";

pub struct PsiSource {
    psi: std::fs::File,
    wake_rd: UnixStream,
    wake_wr: UnixStream,
}

impl PsiSource {
    pub fn new() -> io::Result<PsiSource> {
        let mut psi = OpenOptions::new().read(true).write(true).custom_flags(libc::O_NONBLOCK).open("/proc/pressure/memory")?;
        psi.write_all(TRIGGER.as_bytes())?;
        let (wake_rd, wake_wr) = UnixStream::pair()?;
        wake_rd.set_nonblocking(true)?;
        wake_wr.set_nonblocking(true)?;
        Ok(PsiSource { psi, wake_rd, wake_wr })
    }
}

/// `poll(2)` on two descriptors: `(psi_event, wake)`; `None` on error.
fn poll2(psi: i32, wake: i32, ms: i32) -> Option<(bool, bool)> {
    let mut fds = [libc::pollfd { fd: psi, events: libc::POLLPRI, revents: 0 }, libc::pollfd { fd: wake, events: libc::POLLIN, revents: 0 }];
    // SAFETY: `fds` is a valid array of two pollfd structs for the duration of the call, and the count passed is 2.
    let n = unsafe { libc::poll(fds.as_mut_ptr(), 2, ms) };
    if n < 0 {
        return None;
    }
    Some((fds[0].revents & (libc::POLLPRI | libc::POLLERR) != 0, fds[1].revents & libc::POLLIN != 0))
}

impl MemSource for PsiSource {
    fn snapshot(&self) -> Option<MemInfo> {
        super::sample::parse_meminfo(&std::fs::read_to_string("/proc/meminfo").ok()?)
    }

    fn wait_change(&self, stop: &Stop, timeout: Duration) -> Wake {
        if stop.is_set() {
            return Wake::Stop;
        }
        let ms = timeout.as_millis().min(i32::MAX as u128) as i32;
        match poll2(self.psi.as_raw_fd(), self.wake_rd.as_raw_fd(), ms) {
            Some((_, true)) => Wake::Stop,
            Some((true, _)) => Wake::Event,
            Some(_) => Wake::Timeout,
            // a failed poll must not spin
            None => {
                if stop.sleep(timeout) {
                    Wake::Stop
                } else {
                    Wake::Timeout
                }
            }
        }
    }

    fn wake(&self) {
        let _ = (&self.wake_wr).write(&[1]);
    }

    fn has_events(&self) -> bool {
        true
    }

    fn name(&self) -> &'static str {
        "linux-psi"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::time::Instant;

    #[test]
    fn a_wait_ends_by_deadline() {
        let Ok(s) = PsiSource::new() else { return }; // no PSI here (old kernel, container, permissions)
        let t = Instant::now();
        let w = s.wait_change(&Stop::default(), Duration::from_millis(100));
        assert!(matches!(w, Wake::Timeout | Wake::Event), "{w:?}");
        assert!(t.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn wake_ends_a_long_wait() {
        let Ok(s) = PsiSource::new() else { return };
        let (s, stop) = (Arc::new(s), Arc::new(Stop::default()));
        let (s2, stop2) = (s.clone(), stop.clone());
        let h = std::thread::spawn(move || loop {
            match s2.wait_change(&stop2, Duration::from_secs(60)) {
                Wake::Event | Wake::Timeout => continue,
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
