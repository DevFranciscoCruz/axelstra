//! PTY sessions. Each session owns a pseudo-terminal, a Job Object (so the
//! whole process tree dies with it) and a bounded ring of recent output that
//! lets a UI attach late and replay what it missed.

use crate::job::Job;
use anyhow::{Context, Result};
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use std::collections::VecDeque;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};

const RING_BYTES: usize = 1 << 20;

#[derive(Debug, Clone)]
pub struct Spawn {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub env: Vec<(String, String)>,
    pub cols: u16,
    pub rows: u16,
}

struct Ring {
    buf: VecDeque<u8>,
    subs: Vec<Sender<Vec<u8>>>,
}

struct Shared {
    ring: Mutex<Ring>,
    eof: AtomicBool,
    /// unix ms of the last program output that was not just the echo of our own input
    last_out: AtomicU64,
    /// unix ms of the last input written
    last_in: AtomicU64,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

pub struct Session {
    master: Mutex<Box<dyn MasterPty + Send>>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    child: Mutex<Box<dyn portable_pty::Child + Send + Sync>>,
    job: Job,
    shared: Arc<Shared>,
}

impl Session {
    pub fn spawn(s: &Spawn) -> Result<Session> {
        let pair = native_pty_system()
            .openpty(PtySize { rows: s.rows, cols: s.cols, pixel_width: 0, pixel_height: 0 })?;
        let mut cmd = CommandBuilder::new(&s.program);
        cmd.args(&s.args);
        if let Some(c) = &s.cwd {
            cmd.cwd(c);
        }
        for (k, v) in &s.env {
            cmd.env(k, v);
        }
        let child = pair.slave.spawn_command(cmd).with_context(|| format!("spawn {}", s.program))?;
        drop(pair.slave);

        let job = Job::new()?;
        if let Some(pid) = child.process_id() {
            job.assign_pid(pid)?;
        }

        let mut reader = pair.master.try_clone_reader()?;
        let writer = Arc::new(Mutex::new(pair.master.take_writer()?));
        let shared = Arc::new(Shared {
            ring: Mutex::new(Ring { buf: VecDeque::new(), subs: Vec::new() }),
            eof: AtomicBool::new(false),
            last_out: AtomicU64::new(now_ms()),
            last_in: AtomicU64::new(0),
        });

        // Dedicated drain thread: ConPTY can block on close if output is not read.
        let sh = shared.clone();
        let reply = writer.clone();
        std::thread::spawn(move || {
            // ConPTY asks for the cursor position (ESC[6n) at startup and stalls
            // until answered. Answer the first one here and hide it from the UI,
            // otherwise xterm.js would answer too and the shell would get it twice.
            let mut answered = false;
            let mut chunk = [0u8; 16 * 1024];
            loop {
                match reader.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        let mut data = chunk[..n].to_vec();
                        if !answered {
                            if let Some(i) = data.windows(4).position(|w| w == b"[6n") {
                                data.drain(i..i + 4);
                                answered = true;
                                let mut w = reply.lock().unwrap();
                                let _ = w.write_all(b"[1;1R");
                                let _ = w.flush();
                            }
                        }
                        if data.is_empty() {
                            continue;
                        }
                        // typing echoes back as output: that is the user, not the program working
                        if now_ms().saturating_sub(sh.last_in.load(Ordering::Relaxed)) > 700 {
                            sh.last_out.store(now_ms(), Ordering::Relaxed);
                        }
                        let mut r = sh.ring.lock().unwrap();
                        r.buf.extend(&data);
                        let over = r.buf.len().saturating_sub(RING_BYTES);
                        r.buf.drain(..over);
                        r.subs.retain(|tx| tx.send(data.clone()).is_ok());
                    }
                }
            }
            let mut r = sh.ring.lock().unwrap();
            sh.eof.store(true, Ordering::SeqCst);
            // dropping the senders tells subscribers the stream ended
            r.subs.clear();
        });

        Ok(Session {
            master: Mutex::new(pair.master),
            writer,
            child: Mutex::new(child),
            job,
            shared,
        })
    }

    /// Replay of recent output plus a live stream; no byte is lost or duplicated.
    pub fn subscribe(&self) -> (Vec<u8>, Receiver<Vec<u8>>) {
        let (tx, rx) = channel();
        let mut r = self.shared.ring.lock().unwrap();
        let snap = r.buf.iter().copied().collect();
        if !self.shared.eof.load(Ordering::SeqCst) {
            r.subs.push(tx);
        }
        (snap, rx)
    }

    /// Milliseconds since the program last produced output of its own.
    pub fn idle_ms(&self) -> u64 {
        now_ms().saturating_sub(self.shared.last_out.load(Ordering::Relaxed))
    }

    pub fn write(&self, data: &[u8]) -> Result<()> {
        self.shared.last_in.store(now_ms(), Ordering::Relaxed);
        let mut w = self.writer.lock().unwrap();
        w.write_all(data)?;
        w.flush()?;
        Ok(())
    }

    pub fn resize(&self, cols: u16, rows: u16) -> Result<()> {
        self.master
            .lock()
            .unwrap()
            .resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })?;
        Ok(())
    }

    /// Kills the whole process tree.
    pub fn kill(&self) {
        self.job.kill();
    }

    /// Exit code once the process is gone (None while running).
    pub fn try_wait(&self) -> Option<i32> {
        let st = self.child.lock().unwrap().try_wait().ok().flatten()?;
        Some(st.exit_code() as i32)
    }

    pub fn pid(&self) -> Option<u32> {
        self.child.lock().unwrap().process_id()
    }

    pub fn job_usage(&self) -> (u64, u64) {
        self.job.usage()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// Runs one command line in the platform shell.
    fn spawn_cmd(line: &str) -> Session {
        let (program, args): (&str, Vec<String>) = if cfg!(windows) {
            ("cmd", vec!["/c".into(), line.into()])
        } else {
            ("sh", vec!["-c".into(), line.into()])
        };
        Session::spawn(&Spawn { program: program.into(), args, cwd: None, env: vec![], cols: 80, rows: 24 }).unwrap()
    }

    /// An interactive shell reading from the pty.
    fn spawn_shell() -> Session {
        let (program, args): (&str, Vec<String>) = if cfg!(windows) { ("cmd", vec!["/q".into()]) } else { ("sh", vec![]) };
        Session::spawn(&Spawn { program: program.into(), args, cwd: None, env: vec![], cols: 80, rows: 24 }).unwrap()
    }

    fn wait_for(s: &Session, needle: &str) -> bool {
        let (snap, rx) = s.subscribe();
        let mut all = snap;
        let end = Instant::now() + Duration::from_secs(10);
        while Instant::now() < end {
            if String::from_utf8_lossy(&all).contains(needle) {
                return true;
            }
            if let Ok(d) = rx.recv_timeout(Duration::from_millis(200)) {
                all.extend(d);
            }
        }
        false
    }

    #[test]
    fn captures_output_and_late_subscriber_replays() {
        let s = spawn_cmd("echo hello-axelstra");
        assert!(wait_for(&s, "hello-axelstra"));
        std::thread::sleep(Duration::from_millis(300));
        let (snap, _rx) = s.subscribe();
        assert!(String::from_utf8_lossy(&snap).contains("hello-axelstra"));
    }

    #[test]
    fn input_roundtrip() {
        let s = spawn_shell();
        let line: &[u8] = if cfg!(windows) { b"echo mark&echo er42\r\n" } else { b"echo mark; echo er42\n" };
        s.write(line).unwrap();
        assert!(wait_for(&s, "er42"));
        s.kill();
    }

    #[test]
    fn kill_ends_process_tree() {
        let s = spawn_cmd(if cfg!(windows) { "ping -n 60 127.0.0.1 >nul" } else { "sleep 60; echo done" });
        std::thread::sleep(Duration::from_millis(500));
        assert!(s.try_wait().is_none());
        s.kill();
        let end = Instant::now() + Duration::from_secs(5);
        while Instant::now() < end && s.try_wait().is_none() {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(s.try_wait().is_some(), "process survived kill");
    }

    #[test]
    fn idle_ms_tracks_program_output_not_typing_echo() {
        let s = spawn_shell();
        std::thread::sleep(Duration::from_millis(1500));
        assert!(s.idle_ms() >= 1000, "a quiet shell is idle: {}", s.idle_ms());
        // the shell echoing what we type is not the program working
        s.write(b"x").unwrap();
        std::thread::sleep(Duration::from_millis(400));
        assert!(s.idle_ms() >= 1500, "echo must not count as activity: {}", s.idle_ms());
        s.kill();
    }
}
