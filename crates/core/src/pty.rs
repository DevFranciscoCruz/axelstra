//! PTY sessions. Each session owns a pseudo-terminal, a Job Object (so the
//! whole process tree dies with it) and a bounded ring of recent output that
//! lets a UI attach late and replay what it missed.

use crate::job::Job;
use crate::sync::LockRecover;
use crate::termseq::{DsrFilter, EscTracker};
use anyhow::Result;
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use std::collections::VecDeque;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};

const RING_BYTES: usize = 1 << 20;
/// Chunks (up to 16 KiB each) a subscriber may have waiting before new output is dropped for it.
const SUB_QUEUE: usize = 4096;
/// Input writes waiting for the session's writer thread.
const INPUT_QUEUE: usize = 1024;
/// Silence after which a session counts as quiet (the daemon's "idle" and "went quiet" use the same threshold).
pub const QUIET_MS: u64 = 1500;

/// Runs on the output thread when a quiet session prints again (see `Session::on_wake`).
pub type WakeHook = Arc<dyn Fn() + Send + Sync>;

#[derive(Debug, Clone)]
pub struct Spawn {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub env: Vec<(String, String)>,
    pub cols: u16,
    pub rows: u16,
}

/// Sent to a subscriber that lost output before its stream continues: CAN aborts an escape sequence the lost part left
/// open at the end of what it did receive, and the SGR reset drops colors/attributes that were left on.
const RESYNC: &[u8] = b"\x18\x1b[0m";

struct Sub {
    tx: SyncSender<Vec<u8>>,
    /// output was dropped for this subscriber; it resumes at the next chunk that starts between escape sequences
    lagging: bool,
}

impl Sub {
    /// Hands `data` to the subscriber without ever blocking. False when the receiver is gone.
    fn deliver(&mut self, data: &[u8], starts_clean: bool) -> bool {
        if self.lagging {
            if !starts_clean {
                // the tail of a sequence whose start was lost would print as garbage: skip to a clean boundary
                return true;
            }
            let mut v = Vec::with_capacity(RESYNC.len() + data.len());
            v.extend_from_slice(RESYNC);
            v.extend_from_slice(data);
            return match self.tx.try_send(v) {
                Ok(()) => {
                    self.lagging = false;
                    true
                }
                Err(TrySendError::Full(_)) => true,
                Err(TrySendError::Disconnected(_)) => false,
            };
        }
        match self.tx.try_send(data.to_vec()) {
            Ok(()) => true,
            Err(TrySendError::Full(_)) => {
                self.lagging = true;
                true
            }
            Err(TrySendError::Disconnected(_)) => false,
        }
    }
}

struct Ring {
    buf: VecDeque<u8>,
    subs: Vec<Sub>,
    /// escape-sequence state at the end of everything published so far
    esc: EscTracker,
}

/// A rendered screen tail and the ring generation it was computed from.
struct ScreenCache {
    generation: u64,
    key: (u16, u16, usize, usize),
    lines: Vec<String>,
}

/// Bytes of recent output replayed for the cheap screen view (enough for a status area and the last reply).
const SCREEN_TAIL_BYTES: usize = 16 * 1024;
/// Bytes replayed for the full view (`screen_lines`).
const SCREEN_FULL_BYTES: usize = 256 * 1024;
const SCREEN_CACHE_ENTRIES: usize = 4;

struct Shared {
    ring: Mutex<Ring>,
    eof: AtomicBool,
    /// unix ms of the last program output that was not just the echo of our own input
    last_out: AtomicU64,
    /// unix ms of the last input written
    last_in: AtomicU64,
    /// bumped whenever the ring content changes: a rendered screen is valid while this has not moved
    generation: AtomicU64,
    screens: Mutex<Vec<ScreenCache>>,
    wake: Mutex<Option<WakeHook>>,
}

impl Shared {
    fn new() -> Shared {
        Shared {
            ring: Mutex::new(Ring { buf: VecDeque::new(), subs: Vec::new(), esc: EscTracker::new() }),
            eof: AtomicBool::new(false),
            last_out: AtomicU64::new(now_ms()),
            last_in: AtomicU64::new(0),
            generation: AtomicU64::new(0),
            screens: Mutex::new(Vec::new()),
            wake: Mutex::new(None),
        }
    }

    /// Appends program output to the ring and fans it out to the subscribers.
    fn publish(&self, data: &[u8]) {
        let mut r = self.ring.lock_recover();
        let starts_clean = r.esc.is_ground();
        r.esc.feed(data);
        r.buf.extend(data);
        let over = r.buf.len().saturating_sub(RING_BYTES);
        r.buf.drain(..over);
        self.generation.fetch_add(1, Ordering::SeqCst);
        // a subscriber that stopped reading loses output instead of making us buffer it without bound (the ring still
        // holds the recent part for a fresh subscribe), but never in the middle of an escape sequence; a gone one is dropped
        r.subs.retain_mut(|s| s.deliver(data, starts_clean));
    }

    fn subscribe_with(&self, capacity: usize) -> (Vec<u8>, Receiver<Vec<u8>>) {
        let (tx, rx) = sync_channel(capacity);
        let mut r = self.ring.lock_recover();
        let snap = r.buf.iter().copied().collect();
        if !self.eof.load(Ordering::SeqCst) {
            r.subs.push(Sub { tx, lagging: false });
        }
        (snap, rx)
    }

    /// The last `max` bytes of the ring, and whether older bytes were left out.
    fn tail(&self, max: usize) -> (Vec<u8>, bool) {
        let r = self.ring.lock_recover();
        let skip = r.buf.len().saturating_sub(max);
        (r.buf.iter().skip(skip).copied().collect(), skip > 0)
    }

    /// The last `lines` non-empty lines of the screen after replaying the last `cap` bytes, rendered again only
    /// when the ring changed since the last call with the same arguments (an idle session costs nothing).
    fn screen(&self, size: (u16, u16), lines: usize, cap: usize) -> Vec<String> {
        let key = (size.0, size.1, lines, cap);
        let mut cache = self.screens.lock_recover();
        let generation = self.generation.load(Ordering::SeqCst);
        if let Some(c) = cache.iter().find(|c| c.key == key && c.generation == generation) {
            return c.lines.clone();
        }
        let (mut bytes, cut) = self.tail(cap);
        if cut && cap <= SCREEN_TAIL_BYTES {
            // the cut may have landed inside an escape sequence: the first, partial line is not worth showing
            if let Some(i) = bytes.iter().position(|b| *b == b'\n') {
                bytes.drain(..=i);
            }
        }
        let out = crate::screen::last_lines(&bytes, size.0, size.1, lines);
        cache.retain(|c| c.key != key);
        if cache.len() >= SCREEN_CACHE_ENTRIES {
            cache.remove(0);
        }
        cache.push(ScreenCache { generation, key, lines: out.clone() });
        out
    }

    fn idle_ms(&self) -> u64 {
        now_ms().saturating_sub(self.last_out.load(Ordering::Relaxed))
    }

    fn since_input_ms(&self) -> u64 {
        match self.last_in.load(Ordering::Relaxed) {
            0 => u64::MAX,
            t => now_ms().saturating_sub(t),
        }
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// A cheap, owned handle on a session's output and activity. It holds no registry lock and no session lock: clone
/// it while the session list is locked, drop the lock, then render. Rendering is cached per ring generation.
#[derive(Clone)]
pub struct View {
    shared: Arc<Shared>,
    size: Arc<Mutex<(u16, u16)>>,
}

impl View {
    /// The last `lines` non-empty screen lines from only the last ~16 KiB of output: cheap enough for a list that is
    /// polled every few hundred ms, and free while the session prints nothing.
    pub fn screen_tail(&self, lines: usize) -> Vec<String> {
        self.shared.screen(*self.size.lock_recover(), lines, SCREEN_TAIL_BYTES)
    }

    /// Same from the last 256 KiB (more scrollback, more work; still cached while nothing new arrives).
    pub fn screen_lines(&self, lines: usize) -> Vec<String> {
        self.shared.screen(*self.size.lock_recover(), lines, SCREEN_FULL_BYTES)
    }

    pub fn idle_ms(&self) -> u64 {
        self.shared.idle_ms()
    }

    pub fn since_input_ms(&self) -> u64 {
        self.shared.since_input_ms()
    }

    pub fn tail(&self, max: usize) -> Vec<u8> {
        self.shared.tail(max).0
    }
}

pub struct Session {
    master: Mutex<Box<dyn MasterPty + Send>>,
    child: Mutex<Box<dyn portable_pty::Child + Send + Sync>>,
    job: Job,
    shared: Arc<Shared>,
    size: Arc<Mutex<(u16, u16)>>,
    /// keystrokes wait here for a writer thread of their own, so a session that stops reading its input
    /// (a hung process, a huge paste) never blocks typing into the others
    input: SyncSender<Vec<u8>>,
    /// signalled by the OS when the program exits (see `wait_exit`)
    #[cfg(windows)]
    exit: Option<ExitHandle>,
}

/// A process handle opened only to wait on: the OS signals it when the process ends.
#[cfg(windows)]
struct ExitHandle(windows_sys::Win32::Foundation::HANDLE);
// a process handle can be waited on from any thread
#[cfg(windows)]
unsafe impl Send for ExitHandle {}
#[cfg(windows)]
unsafe impl Sync for ExitHandle {}

#[cfg(windows)]
impl ExitHandle {
    fn open(pid: u32) -> Option<ExitHandle> {
        use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE};
        let h = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
        (!h.is_null()).then_some(ExitHandle(h))
    }
}

#[cfg(windows)]
impl Drop for ExitHandle {
    fn drop(&mut self) {
        unsafe { windows_sys::Win32::Foundation::CloseHandle(self.0) };
    }
}

impl Session {
    pub fn spawn(s: &Spawn) -> Result<Session> {
        let pair = native_pty_system().openpty(PtySize { rows: s.rows, cols: s.cols, pixel_width: 0, pixel_height: 0 })?;
        let mut cmd = CommandBuilder::new(&s.program);
        cmd.args(&s.args);
        if let Some(c) = &s.cwd {
            cmd.cwd(c);
        }
        for (k, v) in &s.env {
            cmd.env(k, v);
        }
        let mut child = pair.slave.spawn_command(cmd).map_err(|e| {
            crate::coded!("session.spawn_failed", "spawn {program}: {detail}", program = s.program, detail = format!("{e:#}"))
        })?;
        drop(pair.slave);
        // opened while portable-pty still holds the child, so the pid cannot have been reused
        #[cfg(windows)]
        let exit = child.process_id().and_then(ExitHandle::open);

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
        let shared = Arc::new(Shared::new());

        // Dedicated drain thread: ConPTY can block on close if output is not read.
        let sh = shared.clone();
        let reply = writer.clone();
        std::thread::spawn(move || {
            // ConPTY asks for the cursor position (ESC[6n) at startup and stalls
            // until answered. Answer the first one here and hide it from the UI,
            // otherwise xterm.js would answer too and the shell would get it twice.
            // The request may arrive split across two reads: DsrFilter holds a possible start back.
            let mut dsr = DsrFilter::new();
            let mut chunk = [0u8; 16 * 1024];
            let note_output = |sh: &Shared| {
                // typing echoes back as output: that is the user, not the program working
                if now_ms().saturating_sub(sh.last_in.load(Ordering::Relaxed)) > 700 {
                    let before = sh.last_out.swap(now_ms(), Ordering::Relaxed);
                    // quiet until now: whoever tracks "went quiet / working again" hears it without polling
                    if now_ms().saturating_sub(before) >= QUIET_MS {
                        let hook = sh.wake.lock_recover().clone();
                        if let Some(f) = hook {
                            f();
                        }
                    }
                }
            };
            loop {
                match reader.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        let (data, answered) = dsr.push(&chunk[..n]);
                        if answered {
                            let mut w = reply.lock_recover();
                            let _ = w.write_all(b"\x1b[1;1R");
                            let _ = w.flush();
                        }
                        if data.is_empty() {
                            continue;
                        }
                        note_output(&sh);
                        sh.publish(&data);
                    }
                }
            }
            let rest = dsr.finish();
            if !rest.is_empty() {
                sh.publish(&rest);
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
            size: Arc::new(Mutex::new((s.cols, s.rows))),
            #[cfg(windows)]
            exit,
        })
    }

    /// Replay of recent output plus a live stream; no byte is lost or duplicated, unless this subscriber falls
    /// `SUB_QUEUE` chunks behind: then output is skipped up to the next point between escape sequences and the
    /// stream continues with a small reset (`RESYNC`), never in the middle of a sequence.
    pub fn subscribe(&self) -> (Vec<u8>, Receiver<Vec<u8>>) {
        self.shared.subscribe_with(SUB_QUEUE)
    }

    /// An owned handle for reading output and activity without touching the session (see `View`).
    pub fn view(&self) -> View {
        View { shared: self.shared.clone(), size: self.size.clone() }
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
        self.shared.generation.fetch_add(1, Ordering::SeqCst);
    }

    /// The last `max` bytes of recent output.
    pub fn tail(&self, max: usize) -> Vec<u8> {
        self.shared.tail(max).0
    }

    /// Milliseconds since the program last produced output of its own.
    pub fn idle_ms(&self) -> u64 {
        self.shared.idle_ms()
    }

    /// `f` runs (on the output thread) each time the program prints after `QUIET_MS` of silence.
    pub fn on_wake(&self, f: WakeHook) {
        *self.shared.wake.lock_recover() = Some(f);
    }

    /// Blocks until the program has exited. No polling: the OS signals the exit (a process handle on Windows,
    /// `waitid` with WNOWAIT on Unix, which leaves the process for `try_wait` to reap). Only when neither is
    /// available does it fall back to checking once a second.
    pub fn wait_exit(&self) {
        #[cfg(windows)]
        if let Some(h) = &self.exit {
            use windows_sys::Win32::System::Threading::{WaitForSingleObject, INFINITE};
            unsafe { WaitForSingleObject(h.0, INFINITE) };
            return;
        }
        #[cfg(unix)]
        if let Some(pid) = self.pid() {
            loop {
                let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
                let r = unsafe { libc::waitid(libc::P_PID, pid as libc::id_t, &mut info, libc::WEXITED | libc::WNOWAIT) };
                if r == 0 {
                    return;
                }
                match std::io::Error::last_os_error().raw_os_error() {
                    Some(libc::EINTR) => continue,
                    // reaped already (by `try_wait`): it has exited
                    Some(libc::ECHILD) => return,
                    _ => break,
                }
            }
        }
        while self.try_wait().is_none() {
            std::thread::sleep(std::time::Duration::from_secs(1));
        }
    }

    /// Milliseconds since the user last typed into this session (u64::MAX if never).
    pub fn since_input_ms(&self) -> u64 {
        self.shared.since_input_ms()
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
            TrySendError::Full(_) => {
                crate::coded!("session.input_full", "session input backlog is full (the program is not reading its input)")
            }
            TrySendError::Disconnected(_) => crate::coded!("session.input_closed", "session input closed"),
        })
    }

    /// The last `lines` non-empty lines as the terminal shows them (the recent output replayed through a terminal
    /// emulator at the session's real size), for reports and `read`.
    pub fn screen_lines(&self, lines: usize) -> Vec<String> {
        self.view().screen_lines(lines)
    }

    /// Like `screen_lines` but replays only the last ~16 KiB: for lists polled every few hundred ms.
    pub fn screen_tail(&self, lines: usize) -> Vec<String> {
        self.view().screen_tail(lines)
    }

    pub fn resize(&self, cols: u16, rows: u16) -> Result<()> {
        *self.size.lock_recover() = (cols, rows);
        self.master.lock_recover().resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })?;
        Ok(())
    }

    /// Kills the whole process tree.
    pub fn kill(&self) {
        self.job.kill();
    }

    /// Exit code once the process is gone (None while running).
    pub fn try_wait(&self) -> Option<i32> {
        let mut child = self.child.lock_recover();
        // unix: look without reaping. The dead leader stays a zombie until the session is dropped, which keeps its pid
        // (the process group id) reserved, so `kill` can still take stragglers of the group without any risk of
        // signalling an unrelated process that got the number; `Job`'s Drop reaps it
        #[cfg(unix)]
        if let Some(pid) = child.process_id() {
            return crate::job::peek_exit(pid);
        }
        let st = child.try_wait().ok().flatten()?;
        Some(st.exit_code() as i32)
    }

    /// True when process `pid` is in this session's process tree (its Job Object on Windows, its process groups and
    /// their descendants on Unix). Used to tell an agent's own processes from the app and the user's terminals.
    pub fn owns_pid(&self, pid: u32) -> bool {
        self.job.contains(pid)
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

    /// `memory_now` / `job_usage` on a process table taken once for the whole poll (see `procs::snapshot`).
    pub fn memory_now_in(&self, snap: &crate::procs::Snapshot) -> u64 {
        self.job.memory_now_in(snap)
    }

    pub fn job_usage_in(&self, snap: &crate::procs::Snapshot) -> (u64, u64) {
        self.job.usage_in(snap)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// Runs one command line in the platform shell.
    fn spawn_cmd(line: &str) -> Session {
        let (program, args): (&str, Vec<String>) =
            if cfg!(windows) { ("cmd", vec!["/c".into(), line.into()]) } else { ("sh", vec!["-c".into(), line.into()]) };
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
        // the tty echoes what is typed, so on Unix the command text must not contain the word its output prints
        let q = if cfg!(windows) { "" } else { "''" };
        s.write(format!("echo typed-a{q}a{eol}").as_bytes()).unwrap();
        s.write_system(format!("echo system-b{q}b{eol}").as_bytes()).unwrap();
        s.write(format!("echo typed-c{q}c{eol}").as_bytes()).unwrap();
        assert!(wait_for(&s, "typed-cc"));
        let out = String::from_utf8_lossy(&s.tail(8192)).into_owned();
        let (a, b, c) = (out.rfind("typed-aa"), out.rfind("system-bb"), out.rfind("typed-cc"));
        assert!(a < b && b < c, "order lost: {out}");
        s.kill();
    }

    #[test]
    fn a_stalled_subscriber_cannot_grow_the_queue_without_bound() {
        let s = spawn_cmd(if cfg!(windows) {
            "for /l %i in (1,1,30000) do @echo line-%i"
        } else {
            "i=0; while [ $i -lt 30000 ]; do echo line-$i; i=$((i+1)); done"
        });
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

    fn drain_all(rx: &Receiver<Vec<u8>>) -> Vec<u8> {
        std::iter::from_fn(|| rx.try_recv().ok()).flatten().collect()
    }

    #[test]
    fn slow_subscriber_resumes_at_a_clean_boundary_with_a_resync() {
        let sh = Shared::new();
        let (_snap, rx) = sh.subscribe_with(1);
        sh.publish(b"a"); // queued
        sh.publish(b"b\x1b[3"); // queue full: dropped, and it opened a sequence
        assert_eq!(drain_all(&rx), b"a");
        sh.publish(b"1mc"); // would print "1mc" as text if delivered: skipped
        sh.publish(b"d");
        let got = drain_all(&rx);
        assert_eq!(got, [RESYNC, b"d"].concat());
        sh.publish(b"e");
        assert_eq!(drain_all(&rx), b"e", "back to normal after the resync");
    }

    #[test]
    fn a_full_queue_never_splits_a_sequence_it_does_deliver() {
        // whatever the subscriber receives, replayed through the tracker, ends between sequences or before a RESYNC
        let sh = Shared::new();
        let (_snap, rx) = sh.subscribe_with(2);
        let stream: Vec<&[u8]> = vec![b"x\x1b[3", b"1;4", b"2mhello", b"\x1b]0;ti", b"tle\x07", b"\x1b[0", b"mdone"];
        let mut got = Vec::new();
        for (i, c) in stream.iter().enumerate() {
            sh.publish(c);
            if i % 3 == 2 {
                got.extend(drain_all(&rx));
            }
        }
        got.extend(drain_all(&rx));
        let mut t = EscTracker::new();
        // feed piecewise on RESYNC boundaries: after each CAN the tracker must be clean
        for part in got.split_inclusive(|b| *b == 0x18) {
            t.feed(part);
            if part.last() == Some(&0x18) {
                assert!(t.is_ground());
            }
        }
        assert!(String::from_utf8_lossy(&got).contains("x"));
    }

    #[test]
    fn dropped_receiver_removes_the_subscriber() {
        let sh = Shared::new();
        let (_snap, rx) = sh.subscribe_with(1);
        drop(rx);
        sh.publish(b"a");
        assert!(sh.ring.lock_recover().subs.is_empty());
    }

    #[test]
    fn screen_is_cached_until_the_ring_changes() {
        let sh = Shared::new();
        sh.publish(b"first\r\n");
        let a = sh.screen((80, 24), 5, SCREEN_TAIL_BYTES);
        assert_eq!(a, vec!["first"]);
        // same generation: the cached answer is returned without replaying (poison the ring behind the cache's back)
        sh.ring.lock_recover().buf.clear();
        assert_eq!(sh.screen((80, 24), 5, SCREEN_TAIL_BYTES), a);
        sh.publish(b"second\r\n");
        assert_eq!(sh.screen((80, 24), 5, SCREEN_TAIL_BYTES), vec!["second"]);
    }

    #[test]
    fn cheap_tail_ignores_old_output_and_a_cut_first_line() {
        let sh = Shared::new();
        for i in 0..4000 {
            sh.publish(format!("old line {i}\r\n").as_bytes());
        }
        sh.publish(b"newest\r\n");
        let lines = sh.screen((80, 24), 3, SCREEN_TAIL_BYTES);
        assert_eq!(lines.last().map(String::as_str), Some("newest"));
        assert!(lines.iter().all(|l| l.starts_with("old line ") || l == "newest"), "{lines:?}");
    }

    #[test]
    fn dsr_split_across_reads_is_still_answered_and_hidden() {
        let mut f = DsrFilter::new();
        let (a, x) = f.push(b"boot\x1b[");
        let (b, y) = f.push(b"6nprompt$ ");
        assert_eq!([a, b].concat(), b"bootprompt$ ");
        assert!(!x && y);
    }

    #[cfg(unix)]
    #[test]
    fn exit_code_is_seen_without_reaping_and_drop_reaps() {
        let s = spawn_cmd("exit 7");
        let pid = s.pid().unwrap() as i32;
        let end = Instant::now() + Duration::from_secs(5);
        while s.try_wait().is_none() && Instant::now() < end {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(s.try_wait(), Some(7));
        assert_eq!(s.try_wait(), Some(7), "repeatable: not reaped by looking");
        drop(s);
        let gone = Instant::now() + Duration::from_secs(3);
        while unsafe { libc::kill(pid, 0) } == 0 && Instant::now() < gone {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_ne!(unsafe { libc::kill(pid, 0) }, 0, "zombie left behind after drop");
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
    fn wait_exit_returns_when_the_program_ends() {
        let s = Arc::new(spawn_cmd(if cfg!(windows) { "ping -n 2 127.0.0.1 >nul" } else { "sleep 1" }));
        let (tx, rx) = std::sync::mpsc::channel();
        let s2 = s.clone();
        std::thread::spawn(move || {
            s2.wait_exit();
            let _ = tx.send(());
        });
        rx.recv_timeout(Duration::from_secs(15)).expect("exit never signalled");
        assert!(s.try_wait().is_some(), "wait_exit returned while the program still ran");
    }

    #[test]
    fn on_wake_fires_when_a_quiet_session_prints_again() {
        let s = spawn_shell();
        let hits = Arc::new(AtomicU64::new(0));
        let h = hits.clone();
        s.on_wake(Arc::new(move || {
            h.fetch_add(1, Ordering::SeqCst);
        }));
        std::thread::sleep(Duration::from_millis(QUIET_MS + 300));
        let line: &[u8] = if cfg!(windows) { b"echo wake-1\r\n" } else { b"echo wake-1\n" };
        s.write_system(line).unwrap();
        let end = Instant::now() + Duration::from_secs(10);
        while Instant::now() < end && hits.load(Ordering::SeqCst) == 0 {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(hits.load(Ordering::SeqCst) >= 1, "no wake after a quiet spell");
        s.kill();
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
