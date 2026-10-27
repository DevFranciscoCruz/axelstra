//! `axelstra lock NAME [--slots N] -- COMMAND`: runs COMMAND once a slot of the named lock is free, so many agents can
//! work at the same time and only the heavy command (a full verify, a build) takes turns. The slots are OS file locks,
//! released when the command ends or the process dies, so a crashed agent never leaves a lock behind.

use anyhow::{bail, Result};
use std::fs::{File, OpenOptions, TryLockError};
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Duration;

/// Ceiling for `--slots`: each slot is a file and, while waiting, a blocked thread.
pub const MAX_SLOTS: usize = 64;

fn dir() -> Result<PathBuf> {
    let d = axelstra_daemon::proto::ensure_data_dir()?.join("locks");
    std::fs::create_dir_all(&d)?;
    Ok(d)
}

fn open_slot(dir: &std::path::Path, name: &str, i: usize) -> std::io::Result<File> {
    OpenOptions::new().create(true).truncate(false).write(true).open(dir.join(format!("{name}-{i}.lock")))
}

/// Waits for a free slot; returns the held file (dropping it releases the slot).
///
/// A free slot is taken at once. Otherwise one thread per slot blocks in the OS lock call, which returns the moment the
/// holder lets go (or dies): no sleeping and retrying. The first thread to get a slot wins; a thread that gets one later
/// lets it go again straight away. The timeout is the wait on the winner's channel.
fn acquire(name: &str, slots: usize, timeout: Duration) -> Result<(File, usize)> {
    let dir = dir()?;
    for i in 0..slots {
        let f = open_slot(&dir, name, i)?;
        match f.try_lock() {
            Ok(()) => return Ok((f, i)),
            Err(TryLockError::WouldBlock) => {}
            Err(TryLockError::Error(e)) => return Err(e.into()),
        }
    }
    if timeout.is_zero() {
        bail!("timed out waiting for lock '{name}'");
    }
    eprintln!("axelstra lock: '{name}' is busy ({slots} at a time), waiting...");
    let claimed = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::channel::<Result<(File, usize)>>();
    for i in 0..slots {
        let (dir, name, claimed, tx) = (dir.clone(), name.to_string(), claimed.clone(), tx.clone());
        std::thread::Builder::new().name(format!("lock-{i}")).spawn(move || {
            let got = open_slot(&dir, &name, i).and_then(|f| f.lock().map(|()| f));
            match got {
                // a slot got after another thread won (or after the timeout) is let go at once
                Ok(f) => {
                    if !claimed.swap(true, Ordering::SeqCst) {
                        let _ = tx.send(Ok((f, i)));
                    }
                }
                Err(e) => {
                    let _ = tx.send(Err(e.into()));
                }
            }
        })?;
    }
    drop(tx);
    match rx.recv_timeout(timeout) {
        Ok(r) => r,
        Err(_) => {
            // a winner may have claimed in the instant between the timeout and here
            if claimed.swap(true, Ordering::SeqCst) {
                if let Ok(r) = rx.recv_timeout(Duration::from_secs(5)) {
                    return r;
                }
            }
            bail!("timed out waiting for lock '{name}'")
        }
    }
}

fn clamp_slots(slots: usize) -> usize {
    slots.clamp(1, MAX_SLOTS)
}

pub fn run(name: &str, slots: usize, timeout: u64, command: &[String]) -> Result<i32> {
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        bail!("lock name: letters, digits, - and _ only");
    }
    if command.is_empty() {
        bail!("no command given");
    }
    let (held, _slot) = acquire(name, clamp_slots(slots), Duration::from_secs(timeout))?;
    let mut cmd = Command::new(&command[0]);
    cmd.args(&command[1..]);
    // If this process dies (killed, its terminal closed), the OS releases the slot at once; the command must not keep
    // running unserialized behind it. On Windows it lives in a kill-on-close Job Object, so it dies with us. On Unix
    // it inherits the lock file (flock belongs to the open file description, so the slot stays held until the command
    // and everything it started have exited) and, on Linux, also gets SIGKILL when we die.
    #[cfg(windows)]
    let (job, mut child) = {
        let job = axelstra_core::job::Job::new()?;
        let child = job.spawn(&mut cmd)?;
        (job, child)
    };
    #[cfg(unix)]
    let mut child = unix::spawn(&mut cmd, &held)?;
    let status = child.wait()?;
    #[cfg(windows)]
    job.kill();
    drop(held);
    Ok(status.code().unwrap_or(1))
}

#[cfg(unix)]
mod unix {
    use super::*;
    use std::os::unix::io::AsRawFd;
    use std::os::unix::process::CommandExt;
    use std::process::Child;

    /// Spawns the command holding the lock file open (not close-on-exec). It stays in our process group and keeps the
    /// terminal: a group of its own would stop on SIGTTIN when it reads stdin.
    pub fn spawn(cmd: &mut Command, held: &File) -> Result<Child> {
        let fd = held.as_raw_fd();
        let parent = unsafe { libc::getpid() };
        // SAFETY: only async-signal-safe calls (fcntl, prctl, getppid) between fork and exec.
        unsafe {
            cmd.pre_exec(move || {
                let flags = libc::fcntl(fd, libc::F_GETFD);
                if flags < 0 || libc::fcntl(fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                #[cfg(any(target_os = "linux", target_os = "android"))]
                {
                    if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL as libc::c_ulong, 0, 0, 0) != 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                    // the lock process may have died between fork and prctl
                    if libc::getppid() != parent {
                        return Err(std::io::Error::other("axelstra lock exited before the command started"));
                    }
                }
                #[cfg(not(any(target_os = "linux", target_os = "android")))]
                let _ = parent;
                Ok(())
            });
        }
        Ok(cmd.spawn()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Locks live under the data dir: point it at a temp dir first, never the real one (docs/TESTING.md).
    fn isolate() {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| {
            let d = std::env::temp_dir().join(format!("axelstra-test-lock-{}", std::process::id()));
            std::fs::create_dir_all(&d).unwrap();
            std::env::set_var("AXELSTRA_DATA_DIR", d);
        });
    }

    /// Removes the lock files a test made (names are per-process, `<name>-<slot>.lock`) so the temp dir stays empty.
    struct Cleanup(String);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            if let Ok(d) = dir() {
                for i in 0..2 {
                    let _ = std::fs::remove_file(d.join(format!("{}-{i}.lock", self.0)));
                }
            }
        }
    }

    #[test]
    fn a_second_holder_waits_until_the_first_lets_go() {
        isolate();
        let name = format!("t{}", std::process::id());
        let _clean = Cleanup(name.clone());
        let (a, _) = acquire(&name, 1, Duration::from_secs(1)).unwrap();
        assert!(acquire(&name, 1, Duration::from_millis(600)).is_err());
        drop(a);
        assert!(acquire(&name, 1, Duration::from_secs(1)).is_ok());
    }

    #[test]
    fn a_waiter_wakes_when_the_holder_lets_go_not_on_a_poll_tick() {
        isolate();
        let name = format!("w{}", std::process::id());
        let _clean = Cleanup(name.clone());
        let (a, _) = acquire(&name, 1, Duration::from_secs(1)).unwrap();
        let releaser = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(150));
            drop(a);
        });
        let t = std::time::Instant::now();
        assert!(acquire(&name, 1, Duration::from_secs(20)).is_ok());
        // the old retry period was 500 ms: a waiter that took the slot sooner was woken by the release
        assert!(t.elapsed() < Duration::from_millis(450), "took {:?}", t.elapsed());
        releaser.join().unwrap();
    }

    #[test]
    fn with_several_slots_the_one_that_frees_is_the_one_taken() {
        isolate();
        let name = format!("m{}", std::process::id());
        let _clean = Cleanup(name.clone());
        let (a, ia) = acquire(&name, 2, Duration::from_secs(1)).unwrap();
        let (b, ib) = acquire(&name, 2, Duration::from_secs(1)).unwrap();
        assert_ne!(ia, ib);
        let releaser = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            drop(b);
        });
        let (_c, ic) = acquire(&name, 2, Duration::from_secs(20)).unwrap();
        assert_eq!(ic, ib);
        releaser.join().unwrap();
        drop(a);
    }

    #[test]
    fn slots_are_capped() {
        assert_eq!(clamp_slots(0), 1);
        assert_eq!(clamp_slots(3), 3);
        assert_eq!(clamp_slots(usize::MAX), MAX_SLOTS);
    }

    #[test]
    fn two_slots_allow_two_holders() {
        isolate();
        let name = format!("s{}", std::process::id());
        let _clean = Cleanup(name.clone());
        let (a, _) = acquire(&name, 2, Duration::from_secs(1)).unwrap();
        let (b, _) = acquire(&name, 2, Duration::from_secs(1)).unwrap();
        assert!(acquire(&name, 2, Duration::from_millis(600)).is_err());
        drop((a, b));
    }
}
