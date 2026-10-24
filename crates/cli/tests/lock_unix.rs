//! R16b: killing `axelstra lock` must not free the slot while its command still runs.
#![cfg(unix)]

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn axelstra(data: &Path) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_axelstra"));
    c.env("HOME", data)
        .env("XDG_DATA_HOME", data)
        .env("AXELSTRA_DATA_DIR", data.join("axelstra"))
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    c
}

/// A second `axelstra lock` that gives up at once: true when it got the slot.
fn probe(data: &Path) -> bool {
    axelstra(data).args(["lock", "r16b", "--timeout", "0", "--", "true"]).status().map(|s| s.success()).unwrap_or(false)
}

fn beats(f: &Path) -> usize {
    std::fs::read_to_string(f).map(|s| s.lines().count()).unwrap_or(0)
}

fn wait_for(what: &str, mut ok: impl FnMut() -> bool) {
    let t = Instant::now();
    while !ok() {
        assert!(t.elapsed() < Duration::from_secs(10), "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn slot_stays_held_after_sigkill_of_the_lock_process_while_the_command_runs() {
    let dir = std::env::temp_dir().join(format!("axelstra-r16b-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let beat = dir.join("beat");
    let pidf = dir.join("pid");
    // heartbeat loop: ends by itself after ~8s so a failing test leaves nothing behind
    let script =
        format!("echo $$ > '{}'; i=0; while [ $i -lt 80 ]; do echo x >> '{}'; i=$((i+1)); sleep 0.1; done", pidf.display(), beat.display());
    let mut holder = axelstra(&dir).args(["lock", "r16b", "--timeout", "5", "--", "sh", "-c", &script]).spawn().unwrap();
    wait_for("the command to start", || beats(&beat) > 1);
    assert!(!probe(&dir), "slot must be busy while the command runs");

    holder.kill().unwrap(); // SIGKILL
    holder.wait().unwrap();
    std::thread::sleep(Duration::from_millis(300));

    // Whenever the command is still beating, a new holder must not get in.
    let before = beats(&beat);
    let got_in = probe(&dir);
    std::thread::sleep(Duration::from_millis(400));
    let alive_during_probe = beats(&beat) > before;
    assert!(!(got_in && alive_during_probe), "second holder ran while the first command was still alive");

    // Once the command is gone the slot is free again.
    let pid = std::fs::read_to_string(&pidf).unwrap().trim().to_string();
    let _ = Command::new("kill").args(["-9", &pid]).status();
    wait_for("the slot to free", || probe(&dir));
    let _ = std::fs::remove_dir_all(&dir);
}
