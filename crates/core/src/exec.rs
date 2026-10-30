//! Run an external program with a deadline, a cap on what it may print and the whole process tree under control.
//!
//! One helper instead of a hand-rolled spawn/wait/drain loop per caller (`git`, `docker`, checks, janitor, publish).
//! The usual bugs of those loops are all handled here:
//!
//! * the child is started inside a [`Job`], so a timeout kills the whole tree (grandchildren such as the real
//!   `docker`/`ssh`/`node` behind a shim included), not just the direct child;
//! * stdout and stderr are drained on their own threads (a full pipe never blocks the child) and each keeps at most
//!   `max_bytes`; the rest is read and discarded so the child never stalls, and `truncated` says it happened;
//! * after the child is gone the drain threads get a short grace period, then are abandoned: a straggler that
//!   escaped the job and still holds the pipe cannot hang the caller;
//! * stdin is `/dev/null`, so nothing waits for input, and on Windows no console window flashes.
//!
//! ```no_run
//! use std::{process::Command, time::Duration};
//! let mut cmd = Command::new("git");
//! cmd.args(["fetch", "--quiet"]).current_dir(".");
//! let out = axelstra_core::exec::run_capped(&mut cmd, Duration::from_secs(30), 1 << 20)?;
//! if out.timed_out { /* the tree was killed */ }
//! # Ok::<(), anyhow::Error>(())
//! ```
//!
//! The caller configures program, args, cwd and env on the `Command`; `run_capped` sets stdio, the Windows creation
//! flags and the process group. A spawn failure is returned as the original `std::io::Error` (check
//! `err.downcast_ref::<std::io::Error>()` for `NotFound`). A non-zero exit or a timeout is **not** an `Err`: look at
//! [`Captured`].

use crate::job::Job;
use crate::sync::LockRecover;
use anyhow::Result;
use std::io::Read;
use std::process::{Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How long the drain threads get to finish once the child (and its tree) is dead.
const DRAIN_GRACE: Duration = Duration::from_secs(2);

/// What a finished (or killed) run produced.
#[derive(Debug, Clone)]
pub struct Captured {
    /// exit status; `None` when the deadline killed it
    pub status: Option<ExitStatus>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub timed_out: bool,
    /// at least one stream had more than `max_bytes`; the excess was dropped
    pub truncated: bool,
}

impl Captured {
    /// Exited on its own with status 0.
    pub fn success(&self) -> bool {
        !self.timed_out && self.status.is_some_and(|s| s.success())
    }

    pub fn stdout_lossy(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }

    pub fn stderr_lossy(&self) -> String {
        String::from_utf8_lossy(&self.stderr).into_owned()
    }
}

struct Drain {
    buf: Arc<Mutex<Vec<u8>>>,
    truncated: Arc<AtomicBool>,
    done: Receiver<()>,
}

fn drain<R: Read + Send + 'static>(mut r: R, max_bytes: usize) -> Drain {
    let buf = Arc::new(Mutex::new(Vec::new()));
    let truncated = Arc::new(AtomicBool::new(false));
    let (tx, done) = channel();
    let (b, t) = (buf.clone(), truncated.clone());
    std::thread::spawn(move || {
        let mut chunk = [0u8; 8192];
        loop {
            match r.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let mut g = b.lock_recover();
                    let room = max_bytes.saturating_sub(g.len());
                    g.extend_from_slice(&chunk[..n.min(room)]);
                    if n > room {
                        t.store(true, Ordering::Relaxed);
                    }
                }
            }
        }
        let _ = tx.send(());
    });
    Drain { buf, truncated, done }
}

impl Drain {
    /// What was read; waits for the end of the stream until `until`, then returns what there is.
    fn collect(self, until: Instant) -> (Vec<u8>, bool) {
        let _ = self.done.recv_timeout(until.saturating_duration_since(Instant::now()));
        let out = std::mem::take(&mut *self.buf.lock_recover());
        (out, self.truncated.load(Ordering::Relaxed))
    }
}

/// Runs `cmd` to completion or until `timeout`, keeping at most `max_bytes` of each of stdout and stderr.
/// See the module docs for the guarantees.
pub fn run_capped(cmd: &mut Command, timeout: Duration, max_bytes: usize) -> Result<Captured> {
    // no window and counted once: `Job::spawn` does both
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let job = Job::new()?;
    let mut child = job.spawn(cmd)?;
    let pid = child.id();
    let out = drain(child.stdout.take().expect("piped stdout"), max_bytes);
    let err = drain(child.stderr.take().expect("piped stderr"), max_bytes);

    // a thread blocked in the OS wait for the child tells us the moment it exits; the deadline is the only timer
    let (done_tx, done_rx) = channel();
    std::thread::spawn(move || {
        let _ = done_tx.send(child.wait().ok());
    });
    let mut timed_out = false;
    let status = match done_rx.recv_timeout(timeout) {
        Ok(st) => st,
        Err(RecvTimeoutError::Timeout) => {
            timed_out = true;
            // the whole tree, leader included (a Job holds every process it started); the waiter reaps the leader
            job.kill();
            let _ = done_rx.recv_timeout(DRAIN_GRACE);
            None
        }
        Err(RecvTimeoutError::Disconnected) => None,
    };
    // stragglers of the finished tree must not keep the pipes open
    job.kill_and_forget(pid);

    let until = Instant::now() + DRAIN_GRACE;
    let (stdout, t1) = out.collect(until);
    let (stderr, t2) = err.collect(until);
    Ok(Captured { status, stdout, stderr, timed_out, truncated: t1 || t2 })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sh(script: &str) -> Command {
        if cfg!(windows) {
            let mut c = Command::new("cmd");
            c.args(["/c", script]);
            c
        } else {
            let mut c = Command::new("sh");
            c.args(["-c", script]);
            c
        }
    }

    #[test]
    fn captures_both_streams_and_status() {
        // cmd's `&` chains; in sh it would background the first echo and race the shell's exit
        let line = if cfg!(windows) { "echo out-1& echo err-1 1>&2" } else { "echo out-1; echo err-1 1>&2" };
        let o = run_capped(&mut sh(line), Duration::from_secs(10), 1 << 16).unwrap();
        assert!(o.success());
        assert!(o.stdout_lossy().contains("out-1"));
        assert!(o.stderr_lossy().contains("err-1"));
        assert!(!o.truncated && !o.timed_out);
    }

    fn children_of(site: &str) -> u64 {
        crate::proc::site_stats().iter().filter(|r| r.site == site).map(|r| r.total).sum()
    }

    #[test]
    fn one_run_counts_one_child_whether_or_not_the_caller_hid_it() {
        let _g = crate::proc::site("test.exec.run_capped_once");
        run_capped(&mut sh("exit 0"), Duration::from_secs(10), 1024).unwrap();
        assert_eq!(children_of("test.exec.run_capped_once"), 1);
        let _g = crate::proc::site("test.exec.hidden_then_run_capped");
        let mut c = sh("exit 0");
        crate::proc::hidden(&mut c);
        run_capped(&mut c, Duration::from_secs(10), 1024).unwrap();
        assert_eq!(children_of("test.exec.hidden_then_run_capped"), 1);
    }

    #[test]
    fn reports_failure_without_erroring() {
        let o = run_capped(&mut sh("exit 3"), Duration::from_secs(10), 1024).unwrap();
        assert!(!o.success() && !o.timed_out);
        assert_eq!(o.status.and_then(|s| s.code()), Some(3));
    }

    #[test]
    fn missing_program_is_an_io_not_found() {
        let e = run_capped(&mut Command::new("axelstra-no-such-program-xyz"), Duration::from_secs(1), 10).unwrap_err();
        assert_eq!(e.downcast_ref::<std::io::Error>().map(|e| e.kind()), Some(std::io::ErrorKind::NotFound));
    }

    #[test]
    fn deadline_kills_and_returns_promptly() {
        let line = if cfg!(windows) { "ping -n 60 127.0.0.1 >nul" } else { "sleep 60" };
        let t = Instant::now();
        let o = run_capped(&mut sh(line), Duration::from_millis(300), 1024).unwrap();
        assert!(o.timed_out && !o.success() && o.status.is_none());
        assert!(t.elapsed() < Duration::from_secs(10), "took {:?}", t.elapsed());
    }

    #[test]
    fn a_quick_exit_returns_at_once_not_at_the_deadline() {
        let t = Instant::now();
        let o = run_capped(&mut sh("exit 0"), Duration::from_secs(120), 1024).unwrap();
        assert!(o.success() && !o.timed_out);
        assert!(t.elapsed() < Duration::from_secs(10), "took {:?}", t.elapsed());
    }

    #[test]
    fn output_is_capped_and_child_does_not_stall() {
        // far more than the pipe buffer: a naive reader that stops at the cap would deadlock the child
        let line = if cfg!(windows) {
            "for /l %i in (1,1,20000) do @echo line-%i-padding-padding-padding"
        } else {
            "i=0; while [ $i -lt 20000 ]; do echo line-$i-padding-padding-padding; i=$((i+1)); done"
        };
        let o = run_capped(&mut sh(line), Duration::from_secs(60), 4096).unwrap();
        assert!(o.success(), "child must run to the end");
        assert_eq!(o.stdout.len(), 4096);
        assert!(o.truncated);
        assert!(o.stdout_lossy().starts_with("line-"));
    }

    #[test]
    fn grandchild_holding_the_pipe_does_not_hang_the_caller() {
        // the shell exits at once; its background child keeps stdout open for a minute
        let line = if cfg!(windows) { "start /b ping -n 60 127.0.0.1 >nul & echo hi" } else { "sleep 60 & echo hi" };
        let t = Instant::now();
        let o = run_capped(&mut sh(line), Duration::from_secs(30), 1024).unwrap();
        assert!(t.elapsed() < Duration::from_secs(10), "took {:?}", t.elapsed());
        assert!(o.stdout_lossy().contains("hi"));
    }
}
