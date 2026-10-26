//! `axelstra lock NAME [--slots N] -- COMMAND`: runs COMMAND once a slot of the named lock is free, so many agents can
//! work at the same time and only the heavy command (a full verify, a build) takes turns. The slots are OS file locks,
//! released when the command ends or the process dies, so a crashed agent never leaves a lock behind.

use anyhow::{bail, Result};
use std::fs::{File, OpenOptions, TryLockError};
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

/// Ceiling for `--slots`: each slot is a file and a polling try-lock per round.
pub const MAX_SLOTS: usize = 64;

fn dir() -> Result<PathBuf> {
    let d = axelstra_daemon::proto::ensure_data_dir()?.join("locks");
    std::fs::create_dir_all(&d)?;
    Ok(d)
}

/// Waits for a free slot; returns the held file (dropping it releases the slot).
fn acquire(name: &str, slots: usize, timeout: Duration) -> Result<(File, usize)> {
    let dir = dir()?;
    let start = Instant::now();
    let mut announced = false;
    loop {
        for i in 0..slots {
            let f = OpenOptions::new().create(true).truncate(false).write(true).open(dir.join(format!("{name}-{i}.lock")))?;
            match f.try_lock() {
                Ok(()) => return Ok((f, i)),
                Err(TryLockError::WouldBlock) => {}
                Err(TryLockError::Error(e)) => return Err(e.into()),
            }
        }
        if start.elapsed() > timeout {
            bail!("timed out waiting for lock '{name}'");
        }
        if !announced {
            eprintln!("axelstra lock: '{name}' is busy ({slots} at a time), waiting...");
            announced = true;
        }
        std::thread::sleep(Duration::from_millis(500));
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
