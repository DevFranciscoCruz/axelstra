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
use std::sync::mpsc::{sync_channel, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

const RING_BYTES: usize = 1 << 20;
/// Chunks (up to 16 KiB each) a subscriber may have waiting before new output is dropped for it.
const SUB_QUEUE: usize = 4096;
/// Input writes waiting for the session's writer thread.
const INPUT_QUEUE: usize = 1024;

/// `lock()` that survives a poisoned mutex: a panic in one handler must not make every later caller panic too.
trait LockRecover<T> {
    fn lock_recover(&self) -> MutexGuard<'_, T>;
}

impl<T> LockRecover<T> for Mutex<T> {
    fn lock_recover(&self) -> MutexGuard<'_, T> {
        self.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

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
    subs: Vec<SyncSender<Vec<u8>>>,
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
    child: Mutex<Box<dyn portable_pty::Child + Send + Sync>>,
    job: Job,
    shared: Arc<Shared>,
    size: Mutex<(u16, u16)>,
    /// keystrokes wait here for a writer thread of their own, so a session that stops reading its input
    /// (a hung process, a huge paste) never blocks typing into the others
    input: SyncSender<Vec<u8>>,
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
        let mut child = pair.slave.spawn_command(cmd).with_context(|| format!("spawn {}", s.program))?;
        drop(pair.slave);

        // the child already runs: if the rest of the setup fails, it must not be left behind with nobody owning it
        let setup = Job::new().and_then(|job| {
            if let Some(pid) = child.process_id() {
                job.assign_pid(pid)?;
            }
            Ok((job, pair.master.try_clone_reader()?, pair.master.take_writer()?))
        });
        let (job, mut reader, writer) = match setup {
            Ok(x) => x,
            Err(e) => {
                let _ = child.kill();
                return Err(e);
            }
        };
        let writer = Arc::new(Mutex::new(writer));
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
                                let mut w = reply.lock_recover();
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
                        let mut r = sh.ring.lock_recover();
                        r.buf.extend(&data);
                        let over = r.buf.len().saturating_sub(RING_BYTES);
                        r.buf.drain(..over);
                        // a subscriber that stopped reading loses output instead of making us buffer it without bound
                        // (the ring still holds the recent part for a fresh subscribe); a gone one is dropped
                        r.subs.retain(|tx| !matches!(tx.try_send(data.clone()), Err(TrySendError::Disconnected(_))));
                    }
                }
            }
            let mut r = sh.ring.lock_recover();
            sh.eof.store(true, Ordering::SeqCst);
            // dropping the senders tells subscribers the stream ended
            r.subs.clear();
        });

        let (input, input_rx) = sync_channel::<Vec<u8>>(INPUT_QUEUE);
        let w = writer.clone();
        std::thread::spawn(move || {
            while let Ok(data) = input_rx.recv() {
                let mut w = w.lock_recover();
                if w.write_all(&data).and_then(|_| w.flush()).is_err() {
                    break;
                }
            }
        });

        Ok(Session {
            master: Mutex::new(pair.master),
            input,
            child: Mutex::new(child),
            job,
            shared,
            size: Mutex::new((s.cols, s.rows)),
        })
    }

    /// Replay of recent output plus a live stream; no byte is lost or duplicated.
    pub fn subscribe(&self) -> (Vec<u8>, Receiver<Vec<u8>>) {
        let (tx, rx) = sync_channel(SUB_QUEUE);
        let mut r = self.shared.ring.lock_recover();
        let snap = r.buf.iter().copied().collect();
        if !self.shared.eof.load(Ordering::SeqCst) {
            r.subs.push(tx);
        }
        (snap, rx)
    }

    /// Puts text in front of everything the session has printed, as if it had come first (the restored
    /// history of a previous run). Not sent to the program.
    pub fn seed(&self, text: &[u8]) {
        let mut r = self.shared.ring.lock_recover();
        for b in text.iter().rev() {
            r.buf.push_front(*b);
        }
        let over = r.buf.len().saturating_sub(RING_BYTES);
        r.buf.drain(..over);
    }

    /// The last `max` bytes of recent output.
    pub fn tail(&self, max: usize) -> Vec<u8> {
        let r = self.shared.ring.lock_recover();
        let skip = r.buf.len().saturating_sub(max);
        r.buf.iter().skip(skip).copied().collect()
    }

    /// Milliseconds since the program last produced output of its own.
    pub fn idle_ms(&self) -> u64 {
        now_ms().saturating_sub(self.shared.last_out.load(Ordering::Relaxed))
    }

    /// Milliseconds since the user last typed into this session (u64::MAX if never).
    pub fn since_input_ms(&self) -> u64 {
        match self.shared.last_in.load(Ordering::Relaxed) {
            0 => u64::MAX,
            t => now_ms().saturating_sub(t),
        }
    }

    /// Writes text that did not come from the user (a notice, a report): it is not counted as typing,
    /// so the output it provokes still reads as the program working.
    pub fn write_system(&self, data: &[u8]) -> Result<()> {
        self.queue_input(data)
    }

    pub fn write(&self, data: &[u8]) -> Result<()> {
        self.shared.last_in.store(now_ms(), Ordering::Relaxed);
        self.queue_input(data)
    }

    /// Everything the program receives goes through the writer thread, in order, so typing and notices never interleave.
    fn queue_input(&self, data: &[u8]) -> Result<()> {
        self.input.try_send(data.to_vec()).map_err(|e| match e {
            TrySendError::Full(_) => anyhow::anyhow!("session input backlog is full (the program is not reading its input)"),
            TrySendError::Disconnected(_) => anyhow::anyhow!("session input closed"),
        })
    }

    /// The last `lines` non-empty lines as the terminal shows them (the recent output replayed through a terminal
    /// emulator at the session's real size), for reports and `read`.
    pub fn screen_lines(&self, lines: usize) -> Vec<String> {
        let (cols, rows) = *self.size.lock_recover();
        crate::screen::last_lines(&self.tail(256 * 1024), cols, rows, lines)
    }

    pub fn resize(&self, cols: u16, rows: u16) -> Result<()> {
        *self.size.lock_recover() = (cols, rows);
        self.master
            .lock_recover()
            .resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })?;
        Ok(())
    }

    /// Kills the whole process tree.
    pub fn kill(&self) {
        self.job.kill();
    }

    /// Exit code once the process is gone (None while running).
    pub fn try_wait(&self) -> Option<i32> {
        let st = self.child.lock_recover().try_wait().ok().flatten()?;
        Some(st.exit_code() as i32)
    }

    pub fn pid(&self) -> Option<u32> {
        self.child.lock_recover().process_id()
    }

    /// Private memory of everything the session runs, right now.
    pub fn memory_now(&self) -> u64 {
        self.job.memory_now()
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
    fn system_writes_share_the_input_queue_with_typing() {
        let s = spawn_shell();
        let eol = if cfg!(windows) { "\r\n" } else { "\n" };
        s.write(format!("echo typed-aa{eol}").as_bytes()).unwrap();
        s.write_system(format!("echo system-bb{eol}").as_bytes()).unwrap();
        s.write(format!("echo typed-cc{eol}").as_bytes()).unwrap();
        assert!(wait_for(&s, "typed-cc"));
        let out = String::from_utf8_lossy(&s.tail(8192)).into_owned();
        let (a, b, c) = (out.rfind("typed-aa"), out.rfind("system-bb"), out.rfind("typed-cc"));
        assert!(a < b && b < c, "order lost: {out}");
        s.kill();
    }

    #[test]
    fn a_stalled_subscriber_cannot_grow_the_queue_without_bound() {
        let s = spawn_cmd(if cfg!(windows) { "for /l %i in (1,1,30000) do @echo line-%i" } else { "i=0; while [ $i -lt 30000 ]; do echo line-$i; i=$((i+1)); done" });
        // subscribed but never read: the producer must still finish and the session stay usable
        let (_snap, rx) = s.subscribe();
        let end = Instant::now() + Duration::from_secs(30);
        while Instant::now() < end && s.try_wait().is_none() {
            std::thread::sleep(Duration::from_millis(100));
        }
        assert!(s.try_wait().is_some(), "output producer stalled behind an unread subscriber");
        let queued = std::iter::from_fn(|| rx.try_recv().ok()).count();
        assert!(queued <= SUB_QUEUE, "{queued} chunks queued");
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
