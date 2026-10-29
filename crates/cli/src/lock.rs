//! `axelstra lock NAME [--slots N] -- COMMAND`: runs COMMAND once a slot of the named lock is free, so many agents can
//! work at the same time and only the heavy command (a full verify, a build) takes turns. The slots are OS file locks,
//! released when the command ends or the process dies, so a crashed agent never leaves a lock behind.
//!
//! `--mem-mb N` swaps the count slots for a memory budget: the holder declares what its command needs and is admitted
//! while the declared needs of the holders fit the budget (setting `build.lock_budget_mb`, else physical memory minus a
//! reserve) AND the memory available right now minus the need still leaves a margin (`LOCK_MARGIN_MB`, setting
//! `build.lock_margin_mb`; `buildenv::mem_gate`). Short memory waits for the next release; with nobody holding, the
//! holder is admitted anyway after a warning. The budget is cut into units of `LOCK_UNIT_MB`, one lock file each (`NAME-mem-<i>.lock`); a holder takes
//! `ceil(N / unit)` free units. A waiter blocks in the OS lock call of one busy unit and rescans when it is released, so
//! there is no polling and nobody holds units while waiting (no deadlock). Use it for every caller of a name or none:
//! count slots and memory units are separate files and do not exclude each other.

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

/// Waits until `need_mb` of the budget is free; returns the held unit files (dropping them releases the memory).
fn acquire_mem(
    name: &str,
    budget_mb: u64,
    need_mb: u64,
    margin_mb: u64,
    available: impl Fn() -> Option<u64>,
    timeout: Duration,
) -> Result<Vec<File>> {
    use axelstra_daemon::buildenv::{admit, mem_gate, units_for, Gate, LOCK_UNIT_MB};
    let dir = dir()?;
    let units = ((budget_mb / LOCK_UNIT_MB).max(1) as usize).min(MAX_SLOTS * 4);
    let need = units_for(need_mb, units);
    let mem_name = format!("{name}-mem");
    let deadline = std::time::Instant::now() + timeout;
    let mut told = false;
    loop {
        let mut files = Vec::with_capacity(units);
        let mut busy = Vec::with_capacity(units);
        for i in 0..units {
            let f = open_slot(&dir, &mem_name, i)?;
            match f.try_lock() {
                Ok(()) => busy.push(false),
                Err(TryLockError::WouldBlock) => busy.push(true),
                Err(TryLockError::Error(e)) => return Err(e.into()),
            }
            files.push(f);
        }
        let free = busy.iter().filter(|b| !**b).count();
        let nobody = free == units;
        let decision = match admit(&busy, need) {
            Ok(take) => match mem_gate(need_mb, need, free, available(), margin_mb, nobody) {
                Gate::Admit => Ok(take),
                Gate::AdmitShort => {
                    eprintln!("axelstra lock: warning: '{name}' needs {need_mb} MB and available memory is short, but nothing else holds it: running anyway");
                    Ok(take)
                }
                // memory is short while others hold units: wait for one of them to let go
                Gate::Wait => Err(busy.iter().position(|b| *b).unwrap_or(0)),
            },
            Err(wait_on) => Err(wait_on),
        };
        match decision {
            // the units not taken are let go (dropped), the taken ones stay held
            Ok(take) => return Ok(files.into_iter().enumerate().filter(|(i, _)| take.contains(i)).map(|(_, f)| f).collect()),
            Err(wait_on) => {
                drop(files);
                if !told {
                    eprintln!("axelstra lock: '{name}' needs {need_mb} MB of a {budget_mb} MB budget, waiting...");
                    told = true;
                }
                let left = deadline.saturating_duration_since(std::time::Instant::now());
                if left.is_zero() {
                    bail!("timed out waiting for lock '{name}'");
                }
                let (tx, rx) = mpsc::channel::<std::io::Result<()>>();
                let (dir, mem_name) = (dir.clone(), mem_name.clone());
                std::thread::Builder::new().name("lock-mem".into()).spawn(move || {
                    // returns the moment the holder lets go; the file is released again and the loop rescans
                    let _ = tx.send(open_slot(&dir, &mem_name, wait_on).and_then(|f| f.lock()));
                })?;
                if rx.recv_timeout(left).is_err() {
                    bail!("timed out waiting for lock '{name}'");
                }
            }
        }
    }
}

/// What a lock of this name declares when the user gave neither `--slots` nor `--mem-mb`: `build` and `verify` are the
/// heavy cargo/npm runs, so they declare the memory of the compile jobs they will use.
fn default_mem_mb(name: &str) -> Option<u64> {
    use axelstra_daemon::buildenv as b;
    if !matches!(name, "build" | "verify") {
        return None;
    }
    let jobs = std::env::var("CARGO_BUILD_JOBS").ok().and_then(|j| j.trim().parse::<u32>().ok()).filter(|j| *j > 0).unwrap_or_else(|| {
        let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
        b::jobs_for(cores, b::memory().map(|m| m.1), b::DEFAULT_PER_JOB_BYTES, b::DEFAULT_RESERVE_BYTES)
    });
    Some(b::mem_mb_for_jobs(jobs))
}

/// `(budget in MB, margin in MB)` from the settings and the machine.
fn budget_and_margin() -> (u64, u64) {
    let file = axelstra_daemon::proto::data_dir().join("settings.json");
    let doc = axelstra_daemon::store::read_json::<serde_json::Value>(&file).ok().flatten().unwrap_or_default();
    let s = axelstra_daemon::buildenv::BuildSettings::from_settings(&doc);
    (
        axelstra_daemon::buildenv::lock_budget_mb(s.lock_budget_mb, axelstra_daemon::buildenv::memory().map(|m| m.0)),
        axelstra_daemon::buildenv::lock_margin_mb(s.lock_margin_mb),
    )
}

fn clamp_slots(slots: usize) -> usize {
    slots.clamp(1, MAX_SLOTS)
}

/// `slots`/`mem_mb` are what the user gave; with neither, `build` and `verify` declare a memory need derived from the jobs.
pub fn run(name: &str, slots: Option<usize>, mem_mb: Option<u64>, timeout: u64, command: &[String]) -> Result<i32> {
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        bail!("lock name: letters, digits, - and _ only");
    }
    if command.is_empty() {
        bail!("no command given");
    }
    let mem = mem_mb.or(if slots.is_none() { default_mem_mb(name) } else { None });
    let held = match mem {
        Some(need) => {
            let (budget, margin) = budget_and_margin();
            acquire_mem(name, budget, need, margin, || axelstra_daemon::buildenv::memory().map(|m| m.1), Duration::from_secs(timeout))?
        }
        None => vec![acquire(name, clamp_slots(slots.unwrap_or(1)), Duration::from_secs(timeout))?.0],
    };
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
    pub fn spawn(cmd: &mut Command, held: &[File]) -> Result<Child> {
        let fds: Vec<_> = held.iter().map(|f| f.as_raw_fd()).collect();
        let parent = unsafe { libc::getpid() };
        // SAFETY: only async-signal-safe calls (fcntl, prctl, getppid) between fork and exec.
        unsafe {
            cmd.pre_exec(move || {
                for &fd in &fds {
                    let flags = libc::fcntl(fd, libc::F_GETFD);
                    if flags < 0 || libc::fcntl(fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) < 0 {
                        return Err(std::io::Error::last_os_error());
                    }
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

    struct CleanupMem(String);
    impl Drop for CleanupMem {
        fn drop(&mut self) {
            if let Ok(d) = dir() {
                for i in 0..8 {
                    let _ = std::fs::remove_file(d.join(format!("{}-mem-{i}.lock", self.0)));
                }
            }
        }
    }

    fn plenty() -> Option<u64> {
        Some(1 << 50)
    }

    #[test]
    fn short_memory_waits_for_a_release_but_never_blocks_the_only_holder() {
        isolate();
        let name = format!("r{}", std::process::id());
        let _clean = CleanupMem(name.clone());
        let budget = 4 * axelstra_daemon::buildenv::LOCK_UNIT_MB;
        let short = || Some(100u64 << 20);
        // nobody holds: admitted despite short memory
        let a = acquire_mem(&name, budget, 512, 2048, short, Duration::from_secs(1)).unwrap();
        // units are free but memory is short and `a` holds: waits
        assert!(acquire_mem(&name, budget, 512, 2048, short, Duration::from_millis(400)).is_err());
        let releaser = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(150));
            drop(a);
        });
        // the release wakes it, and with nobody holding it is admitted
        assert!(acquire_mem(&name, budget, 512, 2048, short, Duration::from_secs(20)).is_ok());
        releaser.join().unwrap();
    }

    #[test]
    fn memory_budget_admits_what_fits_and_wakes_on_release() {
        isolate();
        let name = format!("g{}", std::process::id());
        let _clean = CleanupMem(name.clone());
        // 4 units; two holders of 2 units fill it
        let budget = 4 * axelstra_daemon::buildenv::LOCK_UNIT_MB;
        let a = acquire_mem(&name, budget, 1024, 0, plenty, Duration::from_secs(1)).unwrap();
        let b = acquire_mem(&name, budget, 1024, 0, plenty, Duration::from_secs(1)).unwrap();
        assert_eq!((a.len(), b.len()), (2, 2));
        assert!(acquire_mem(&name, budget, 512, 0, plenty, Duration::from_millis(400)).is_err());
        let releaser = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(150));
            drop(a);
        });
        let t = std::time::Instant::now();
        assert!(acquire_mem(&name, budget, 1024, 0, plenty, Duration::from_secs(20)).is_ok());
        assert!(t.elapsed() < Duration::from_secs(5), "took {:?}", t.elapsed());
        releaser.join().unwrap();
        drop(b);
    }

    #[test]
    fn a_need_above_the_budget_still_runs_alone() {
        isolate();
        let name = format!("h{}", std::process::id());
        let _clean = CleanupMem(name.clone());
        let held = acquire_mem(&name, 1024, 99999, 0, plenty, Duration::from_secs(1)).unwrap();
        assert_eq!(held.len(), 2);
    }

    #[test]
    fn only_build_and_verify_declare_a_default_need() {
        assert!(default_mem_mb("verify").is_some());
        assert!(default_mem_mb("build").is_some());
        assert!(default_mem_mb("deploy").is_none());
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
