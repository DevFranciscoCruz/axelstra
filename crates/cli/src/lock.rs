//! `axelstra lock NAME [--slots N] -- COMMAND`: runs COMMAND once a slot of the named lock is free, so many agents can
//! work at the same time and only the heavy command (a full verify, a build) takes turns. The slots are OS file locks,
//! released when the command ends or the process dies, so a crashed agent never leaves a lock behind.

use anyhow::{bail, Result};
use std::fs::{File, OpenOptions, TryLockError};
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

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

pub fn run(name: &str, slots: usize, timeout: u64, command: &[String]) -> Result<i32> {
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        bail!("lock name: letters, digits, - and _ only");
    }
    let (_held, _slot) = acquire(name, slots.max(1), Duration::from_secs(timeout))?;
    let status = Command::new(&command[0]).args(&command[1..]).status()?;
    Ok(status.code().unwrap_or(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_holder_waits_until_the_first_lets_go() {
        let name = format!("t{}", std::process::id());
        let (a, _) = acquire(&name, 1, Duration::from_secs(1)).unwrap();
        assert!(acquire(&name, 1, Duration::from_millis(600)).is_err());
        drop(a);
        assert!(acquire(&name, 1, Duration::from_secs(1)).is_ok());
    }

    #[test]
    fn two_slots_allow_two_holders() {
        let name = format!("s{}", std::process::id());
        let (a, _) = acquire(&name, 2, Duration::from_secs(1)).unwrap();
        let (b, _) = acquire(&name, 2, Duration::from_secs(1)).unwrap();
        assert!(acquire(&name, 2, Duration::from_millis(600)).is_err());
        drop((a, b));
    }
}
