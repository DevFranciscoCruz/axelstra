//! Blocking client (named pipe opened as a plain file on Windows, Unix socket elsewhere),
//! so callers need no async runtime.

use crate::proto::*;
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
#[cfg(windows)]
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::process::{Child, ChildStdin, ChildStdout};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

pub enum Frame {
    Output { session: u32, data: Vec<u8> },
    Event(Value),
    Response(Value),
}

/// One connection, cloneable so a second thread can read while another writes.
enum Conn {
    #[cfg(windows)]
    File(File),
    #[cfg(unix)]
    Unix(std::os::unix::net::UnixStream),
    /// A child process (`ssh`, `wsl.exe`) running `axelstra bridge` on the other machine; its stdin/stdout carry the frames.
    Child(ChildConn),
}

fn recover<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The child process behind a `Conn::Child`, shared by every clone of the connection and by the watchdog.
struct ChildShared {
    child: Mutex<Child>,
    /// the last bytes the child wrote to stderr (ssh and wsl.exe explain their failures there)
    stderr: Arc<Mutex<Vec<u8>>>,
    label: String,
}

/// Longest stderr tail kept from the child.
const STDERR_KEEP: usize = 4096;

impl ChildShared {
    fn kill(&self) {
        let _ = recover(&self.child).kill();
    }

    /// What the child said and how it ended, for an error message; `None` when it said nothing and still runs.
    fn diagnostics(&self) -> Option<String> {
        let end = std::time::Instant::now() + Duration::from_millis(300);
        let status = loop {
            let st = recover(&self.child).try_wait().ok().flatten();
            if st.is_some() || std::time::Instant::now() >= end {
                break st;
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        if status.is_some() {
            // let the stderr reader take the last bytes
            std::thread::sleep(Duration::from_millis(50));
        }
        // wsl.exe writes its own messages as UTF-16LE: dropping the NULs leaves the ASCII part readable
        let raw: Vec<u8> = recover(&self.stderr).iter().copied().filter(|b| *b != 0).collect();
        let said = String::from_utf8_lossy(&raw).trim().to_string();
        match (status, said.is_empty()) {
            (Some(st), true) => Some(format!("{} ended ({st})", self.label)),
            (Some(st), false) => Some(format!("{} ended ({st}) and said: {said}", self.label)),
            (None, false) => Some(format!("{} said: {said}", self.label)),
            (None, true) => None,
        }
    }
}

impl Drop for ChildShared {
    fn drop(&mut self) {
        let c = self.child.get_mut().unwrap_or_else(PoisonError::into_inner);
        let _ = c.kill();
        let _ = c.wait();
    }
}

#[derive(Clone)]
struct ChildConn {
    rd: Arc<Mutex<ChildStdout>>,
    wr: Arc<Mutex<ChildStdin>>,
    shared: Arc<ChildShared>,
}

impl Conn {
    fn try_clone(&self) -> std::io::Result<Conn> {
        Ok(match self {
            #[cfg(windows)]
            Conn::File(f) => Conn::File(f.try_clone()?),
            #[cfg(unix)]
            Conn::Unix(s) => Conn::Unix(s.try_clone()?),
            Conn::Child(c) => Conn::Child(c.clone()),
        })
    }
}

impl Read for Conn {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            #[cfg(windows)]
            Conn::File(f) => f.read(buf),
            #[cfg(unix)]
            Conn::Unix(s) => s.read(buf),
            Conn::Child(c) => recover(&c.rd).read(buf),
        }
    }
}

impl Write for Conn {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            #[cfg(windows)]
            Conn::File(f) => f.write(buf),
            #[cfg(unix)]
            Conn::Unix(s) => s.write(buf),
            Conn::Child(c) => recover(&c.wr).write(buf),
        }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            #[cfg(windows)]
            Conn::File(f) => f.flush(),
            #[cfg(unix)]
            Conn::Unix(s) => s.flush(),
            Conn::Child(c) => recover(&c.wr).flush(),
        }
    }
}

/// An error the daemon itself returned (the call arrived and was refused), as opposed to a broken connection:
/// retrying such a call would only repeat the refusal, or worse, repeat a side effect.
#[derive(Debug)]
pub struct Remote(pub String, pub Option<axelstra_core::msg::Coded>);

impl std::fmt::Display for Remote {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Remote {
    /// the message code the daemon sent, so `msg::find` sees it through this wrapper
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.1.as_ref().map(|c| c as &(dyn std::error::Error + 'static))
    }
}

/// The request could not be written to the daemon, so it never ran: retrying it is always safe.
#[derive(Debug)]
pub struct NotSent(pub std::io::Error);

impl std::fmt::Display for NotSent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "request not sent: {}", self.0)
    }
}

impl std::error::Error for NotSent {}

/// The daemon did not answer a request in time. The connection is unusable afterwards (a late reply could
/// arrive half way through the next one), so the caller must reconnect.
#[derive(Debug)]
pub struct CallTimeout(pub String, pub Option<axelstra_core::msg::Coded>);

impl std::fmt::Display for CallTimeout {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for CallTimeout {
    /// the message code naming the method that took long, so `msg::find` sees it through this wrapper
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.1.as_ref().map(|c| c as &(dyn std::error::Error + 'static))
    }
}

/// A call that ran into a limit. `rpc.call_timeout {method, secs}` is a limit chosen as policy (an explicit `Client::timeout`,
/// or the cosmetic methods of `policy_limit`); `rpc.call_ceiling {method, secs}` is the high safety ceiling for a lost
/// handle. Either way the UI shows which call it was, in its language, instead of a global "the daemon did not answer".
fn call_timeout(method: &str, limit: Duration, text: String, ceiling: bool) -> CallTimeout {
    let secs = limit.as_secs().max(1);
    let coded = if ceiling {
        axelstra_core::coded!(
            "rpc.call_ceiling",
            "{method} got no reply within the safety ceiling of {secs} s; the connection was dropped",
            method = method,
            secs = secs
        )
    } else {
        axelstra_core::coded!("rpc.call_timeout", "{method} did not answer within {secs} s", method = method, secs = secs)
    }
    .downcast::<axelstra_core::msg::Coded>()
    .ok();
    CallTimeout(text, coded)
}

/// The daemon (or the `ssh`/`wsl.exe` child that carries the connection) went away while a call was pending: the event
/// that ends a call, with no waiting on a clock.
fn connection_closed(method: &str) -> anyhow::Error {
    axelstra_core::coded!("rpc.connection_closed", "{method}: the daemon closed the connection", method = method)
}

/// A pending call never fails by elapsed time: it ends with the reply or with the connection closing. This is only the
/// safety ceiling for a handle that lost its peer without telling (10 min by default, `AXELSTRA_CALL_CEILING_SECS` to
/// change it, at least 1 s).
pub const DEFAULT_CEILING_SECS: u64 = 600;

pub fn ceiling() -> Duration {
    static CEILING: std::sync::OnceLock<Duration> = std::sync::OnceLock::new();
    *CEILING.get_or_init(|| {
        let secs =
            std::env::var("AXELSTRA_CALL_CEILING_SECS").ok().and_then(|v| v.trim().parse::<u64>().ok()).unwrap_or(DEFAULT_CEILING_SECS);
        Duration::from_secs(secs.max(1))
    })
}

/// A limit chosen per method as policy, not as a watchdog: only cosmetic reads (a project logo) that the UI drops silently
/// when they are slow. Every other method has none, so it ends by event (the reply, the connection closing) or, as a last
/// resort, at the ceiling.
pub fn policy_limit(method: &str) -> Option<Duration> {
    match method {
        "project.logo" => Some(Duration::from_secs(8)),
        _ => None,
    }
}

/// Methods that may run long (pushes, merges, clones, whole agent runs), listed because they are kept off the quick lane.
const LONG_METHODS: &[&str] = &[
    "git.push",
    "git.pr",
    "git.suggest",
    "git.commit",
    "worktree.create",
    "worktree.from_pr",
    "worktree.sweep",
    "project.add",
    "project.remove",
    "project.init",
    "project.clone",
    "automation.run",
    "orch.start",
    "gh.list",
    "gh.comments",
    "gh.send_comment",
    "git.suggest_pr",
    "agent.search",
    "git.merge",
    "git.checks",
    "diagnostics",
];

/// True for methods that may run long (git on many worktrees, gh, docker, scans): the app keeps them off the connection
/// reserved for quick calls (`Lanes`), so a slow scan never holds up `project.list` or `session.list`.
pub fn is_slow(method: &str) -> bool {
    const SLOW_PREFIX: &[&str] = &["git.", "gh.", "docker.", "pm.", "orch.", "worktree.", "dispatch.", "ports.", "fs."];
    // `project.list` and the like read the project file and must stay quick; the project operations that run git or clone do not
    SLOW_PREFIX.iter().any(|p| method.starts_with(p)) || LONG_METHODS.contains(&method) || method == "watch.subscribe"
}

/// True when `e` is the end of the stream or a reset: the peer is gone.
fn is_closed(e: &anyhow::Error) -> bool {
    use std::io::ErrorKind::*;
    e.chain()
        .filter_map(|c| c.downcast_ref::<std::io::Error>())
        .any(|io| matches!(io.kind(), UnexpectedEof | BrokenPipe | ConnectionReset | ConnectionAborted) || io.raw_os_error() == Some(109))
}

/// True for the error a read gets when its deadline fires: a socket timeout (Unix) or the pipe read cancelled by
/// the watchdog (Windows, ERROR_OPERATION_ABORTED).
fn is_timeout(e: &anyhow::Error) -> bool {
    e.chain()
        .filter_map(|c| c.downcast_ref::<std::io::Error>())
        .any(|io| matches!(io.kind(), std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock) || io.raw_os_error() == Some(995))
}

/// One watchdog thread for every deadline of the process: calls register `(what to do, time)` and the thread acts on the
/// ones that run out: it cancels the pending I/O of a pipe handle (Windows) or kills the child process behind a bridge.
/// (It used to be one OS thread per call.)
mod watchdog {
    use std::sync::{Arc, Condvar, Mutex, Once};
    use std::time::{Duration, Instant};

    enum Action {
        #[cfg(windows)]
        Cancel(isize),
        Kill(Arc<super::ChildShared>),
    }

    struct Entry {
        id: u64,
        at: Instant,
        action: Action,
    }

    struct State {
        entries: Vec<Entry>,
        next: u64,
    }

    static STATE: Mutex<State> = Mutex::new(State { entries: Vec::new(), next: 0 });
    static WAKE: Condvar = Condvar::new();
    static START: Once = Once::new();

    fn lock() -> std::sync::MutexGuard<'static, State> {
        STATE.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    #[cfg(windows)]
    pub fn arm(handle: isize, limit: Duration) -> u64 {
        push(Action::Cancel(handle), limit)
    }

    pub fn arm_kill(child: Arc<super::ChildShared>, limit: Duration) -> u64 {
        push(Action::Kill(child), limit)
    }

    fn push(action: Action, limit: Duration) -> u64 {
        START.call_once(|| {
            std::thread::Builder::new().name("client-watchdog".into()).spawn(run).expect("watchdog thread");
        });
        let mut st = lock();
        st.next += 1;
        let id = st.next;
        st.entries.push(Entry { id, at: Instant::now() + limit, action });
        drop(st);
        WAKE.notify_all();
        id
    }

    /// Returns true when the deadline had already fired (the I/O was cancelled, the child killed).
    pub fn disarm(id: u64) -> bool {
        let mut st = lock();
        let before = st.entries.len();
        st.entries.retain(|e| e.id != id);
        st.entries.len() == before
    }

    fn run() {
        let mut st = lock();
        loop {
            let now = Instant::now();
            // acting happens under the lock, so `disarm` cannot return while a cancel is about to land on a later
            // call's I/O on the same handle
            st.entries.retain(|e| {
                if e.at <= now {
                    match &e.action {
                        #[cfg(windows)]
                        Action::Cancel(handle) => {
                            unsafe { windows_sys::Win32::System::IO::CancelIoEx(*handle as _, std::ptr::null()) };
                        }
                        Action::Kill(child) => child.kill(),
                    }
                    false
                } else {
                    true
                }
            });
            st = match st.entries.iter().map(|e| e.at).min() {
                Some(next) => {
                    WAKE.wait_timeout(st, next.saturating_duration_since(Instant::now()))
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .0
                }
                None => WAKE.wait(st).unwrap_or_else(std::sync::PoisonError::into_inner),
            };
        }
    }
}

/// Bounds the blocking I/O on one connection handle until `finish`.
enum Deadline {
    #[cfg(windows)]
    Cancel(u64),
    #[cfg(unix)]
    Sock(std::os::unix::net::UnixStream),
    /// a bridge child: when the deadline runs out the child is killed, which ends the pending read or write
    Kill(u64),
}

impl Deadline {
    fn start(conn: &Conn, limit: Duration) -> std::io::Result<Deadline> {
        Ok(match conn {
            #[cfg(windows)]
            Conn::File(f) => {
                use std::os::windows::io::AsRawHandle;
                Deadline::Cancel(watchdog::arm(f.as_raw_handle() as isize, limit))
            }
            #[cfg(unix)]
            Conn::Unix(s) => {
                let sock = s.try_clone()?;
                sock.set_read_timeout(Some(limit))?;
                sock.set_write_timeout(Some(limit))?;
                Deadline::Sock(sock)
            }
            Conn::Child(c) => Deadline::Kill(watchdog::arm_kill(c.shared.clone(), limit)),
        })
    }

    /// Ends the bound; true when it had already fired.
    fn finish(self) -> bool {
        match self {
            #[cfg(windows)]
            Deadline::Cancel(id) => watchdog::disarm(id),
            #[cfg(unix)]
            Deadline::Sock(sock) => {
                let _ = sock.set_read_timeout(None);
                let _ = sock.set_write_timeout(None);
                false
            }
            Deadline::Kill(id) => watchdog::disarm(id),
        }
    }
}

pub struct Client {
    rd: Conn,
    wr: Conn,
    next: u64,
    /// protocol version the daemon reported in `hello`
    pub proto: u64,
    /// which daemon run answered `hello`; a different value on reconnect means the daemon restarted
    pub instance: u64,
    /// when set, every `call` is limited to this long (an explicit choice of the caller); else only `policy_limit` and `ceiling` apply
    pub timeout: Option<Duration>,
    /// the safety ceiling for a call with no other limit (see `ceiling()`)
    pub ceiling: Duration,
    /// a call timed out: the stream may hold a late reply, so this connection must not be reused
    broken: bool,
    /// build id the daemon reported in `hello` (empty for a daemon from before it existed)
    pub build: String,
    /// `SECURITY_MIN` of the daemon (0 for a daemon from before it existed)
    pub security: u64,
    /// version, short commit and build date the daemon reported in `hello` (empty for a daemon from before they existed)
    pub version: String,
    pub commit: String,
    pub built: String,
    /// the whole `hello` result, for the bridge to hand on to its own client
    hello: Value,
}

#[cfg(windows)]
fn open(pipe: &str) -> Result<Conn> {
    match open_pipe(pipe) {
        // an older daemon (still running after an update) listens on the pre-SID name
        Err(e) if pipe == pipe_name() && is_not_found(&e) => open_pipe(&legacy_pipe_name()).map_err(|_| e),
        r => r,
    }
}

#[cfg(windows)]
fn is_not_found(e: &anyhow::Error) -> bool {
    e.chain().filter_map(|c| c.downcast_ref::<std::io::Error>()).any(|io| io.kind() == std::io::ErrorKind::NotFound)
}

#[cfg(windows)]
fn open_pipe(pipe: &str) -> Result<Conn> {
    use std::os::windows::fs::OpenOptionsExt;
    // SECURITY_IDENTIFICATION: whoever serves the pipe can learn who we are but not act as us
    const SECURITY_IDENTIFICATION: u32 = 0x0001_0000;
    let mut tries = 0;
    loop {
        match OpenOptions::new().read(true).write(true).security_qos_flags(SECURITY_IDENTIFICATION).open(pipe) {
            Ok(f) => {
                // the token goes to the pipe only after its server proved to be one of our own processes
                crate::pipesec::verify_server_owner(&f).with_context(|| format!("not trusting the pipe {pipe}"))?;
                return Ok(Conn::File(f));
            }
            // ERROR_PIPE_BUSY: the daemon is between instances
            Err(e) if e.raw_os_error() == Some(231) && tries < 20 => {
                tries += 1;
                // the OS wakes us the moment an instance is free (bounded at 50 ms per try)
                use std::os::windows::ffi::OsStrExt;
                let wide: Vec<u16> = std::ffi::OsStr::new(pipe).encode_wide().chain(std::iter::once(0)).collect();
                unsafe { windows_sys::Win32::System::Pipes::WaitNamedPipeW(wide.as_ptr(), 50) };
            }
            Err(e) => return Err(e).with_context(|| format!("daemon not reachable at {pipe}")),
        }
    }
}

/// The executable of the process at the other end of a unix socket.
#[cfg(unix)]
fn peer_exe(s: &std::os::unix::net::UnixStream) -> Option<std::path::PathBuf> {
    use std::os::fd::AsRawFd;
    let fd = s.as_raw_fd();
    #[cfg(target_os = "linux")]
    {
        let mut cred = libc::ucred { pid: 0, uid: 0, gid: 0 };
        let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
        let ok = unsafe { libc::getsockopt(fd, libc::SOL_SOCKET, libc::SO_PEERCRED, (&mut cred as *mut libc::ucred).cast(), &mut len) };
        if ok != 0 || cred.pid <= 0 {
            return None;
        }
        std::fs::read_link(format!("/proc/{}/exe", cred.pid)).ok()
    }
    #[cfg(target_os = "macos")]
    {
        let mut pid: libc::pid_t = 0;
        let mut len = std::mem::size_of::<libc::pid_t>() as libc::socklen_t;
        let ok = unsafe { libc::getsockopt(fd, libc::SOL_LOCAL, libc::LOCAL_PEERPID, (&mut pid as *mut libc::pid_t).cast(), &mut len) };
        if ok != 0 || pid <= 0 {
            return None;
        }
        let mut buf = vec![0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
        let n = unsafe { libc::proc_pidpath(pid, buf.as_mut_ptr().cast(), buf.len() as u32) };
        if n <= 0 {
            return None;
        }
        buf.truncate(n as usize);
        Some(std::path::PathBuf::from(String::from_utf8(buf).ok()?))
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = fd;
        None
    }
}

#[cfg(unix)]
fn open(path: &str) -> Result<Conn> {
    let s = std::os::unix::net::UnixStream::connect(path).with_context(|| format!("daemon not reachable at {path}"))?;
    Ok(Conn::Unix(s))
}

/// A read that never leaves a blocking `ReadFile` pending on a pipe file. On Windows a synchronous handle runs one
/// operation at a time: a pending read keeps a write on the (duplicated) handle waiting, so a relay that reads in one
/// thread and writes in another would stall on the first request. This waits for data with `PeekNamedPipe` instead.
#[cfg(windows)]
struct PeekRead(Conn);

#[cfg(windows)]
impl Read for PeekRead {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::System::Pipes::PeekNamedPipe;
        let Conn::File(f) = &self.0 else { return self.0.read(buf) };
        let mut nap = Duration::from_millis(1);
        loop {
            let mut avail = 0u32;
            let ok = unsafe {
                PeekNamedPipe(f.as_raw_handle() as _, std::ptr::null_mut(), 0, std::ptr::null_mut(), &mut avail, std::ptr::null_mut())
            };
            if ok == 0 {
                // broken pipe: the daemon closed its end, which reads as end of stream
                return Ok(0);
            }
            if avail > 0 {
                let n = buf.len().min(avail as usize);
                return self.0.read(&mut buf[..n]);
            }
            // kept: a synchronous pipe handle has no readiness event (a pending ReadFile would block writes), so a short backoff
            std::thread::sleep(nap);
            nap = (nap * 2).min(Duration::from_millis(10));
        }
    }
}

fn spawn_child(program: &str, args: &[String], env: &[(String, String)], label: &str) -> Result<ChildConn> {
    use std::process::{Command, Stdio};
    let mut cmd = Command::new(program);
    cmd.args(args).envs(env.iter().map(|(k, v)| (k, v))).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    // the app has no console, ssh/wsl must not flash one
    axelstra_core::proc::hidden(&mut cmd);
    let mut child = cmd.spawn().map_err(|e| {
        axelstra_core::coded!(
            "host.spawn_failed",
            "could not start `{program}` for {label}: {detail}",
            program = program,
            label = label,
            detail = e
        )
    })?;
    let (stdin, stdout, stderr) = (child.stdin.take(), child.stdout.take(), child.stderr.take());
    let (Some(stdin), Some(stdout), Some(mut stderr)) = (stdin, stdout, stderr) else {
        let _ = child.kill();
        let _ = child.wait();
        return Err(axelstra_core::coded!("host.attach_failed", "could not attach to the pipes of `{program}`", program = program));
    };
    let said = Arc::new(Mutex::new(Vec::new()));
    let tail = said.clone();
    std::thread::Builder::new().name("remote-stderr".into()).spawn(move || {
        let mut buf = [0u8; 1024];
        while let Ok(n) = stderr.read(&mut buf) {
            if n == 0 {
                break;
            }
            let mut t = recover(&tail);
            t.extend_from_slice(&buf[..n]);
            if t.len() > STDERR_KEEP {
                let cut = t.len() - STDERR_KEEP;
                t.drain(..cut);
            }
        }
    })?;
    let shared = Arc::new(ChildShared { child: Mutex::new(child), stderr: said, label: label.to_string() });
    Ok(ChildConn { rd: Arc::new(Mutex::new(stdout)), wr: Arc::new(Mutex::new(stdin)), shared })
}

impl Client {
    pub fn connect(pipe: &str, token: &str) -> Result<Client> {
        Self::connect_as(pipe, token, None)
    }

    /// For the CLI, which agents run: the daemon refuses to let such a connection turn screen control on.
    pub fn connect_cli(pipe: &str, token: &str) -> Result<Client> {
        Self::connect_as(pipe, token, Some("cli"))
    }

    /// As an agent session does: presents the session's own credential (`AXELSTRA_AGENT_TOKEN`), which the daemon
    /// answers with role=agent. The app-only calls are refused on such a connection.
    pub fn connect_agent(pipe: &str, token: &str, agent_token: &str) -> Result<Client> {
        Self::connect_with(pipe, token, Some("cli"), Some(agent_token.to_string()))
    }

    fn connect_as(pipe: &str, token: &str, client: Option<&str>) -> Result<Client> {
        let agent = if client == Some("cli") { std::env::var("AXELSTRA_AGENT_TOKEN").ok().filter(|t| !t.is_empty()) } else { None };
        Self::connect_with(pipe, token, client, agent)
    }

    fn connect_with(pipe: &str, token: &str, client: Option<&str>, agent_token: Option<String>) -> Result<Client> {
        let f = open(pipe)?;
        let mut c = Client {
            rd: f.try_clone()?,
            wr: f,
            next: 0,
            proto: 0,
            instance: 0,
            timeout: None,
            ceiling: ceiling(),
            broken: false,
            build: String::new(),
            security: 0,
            version: String::new(),
            commit: String::new(),
            built: String::new(),
            hello: Value::Null,
        };
        let hello = c.call("hello", json!({"token": token, "client": client, "agent_token": agent_token}))?;
        c.adopt_hello(hello);
        Ok(c)
    }

    fn adopt_hello(&mut self, hello: Value) {
        let c = self;
        c.proto = hello["proto"].as_u64().unwrap_or(1);
        c.instance = hello["instance"].as_u64().unwrap_or(0);
        c.build = hello["build"].as_str().unwrap_or_default().to_string();
        c.security = hello["security"].as_u64().unwrap_or(0);
        let text = |k: &str| hello[k].as_str().unwrap_or_default().to_string();
        (c.version, c.commit, c.built) = (text("version"), text("commit"), text("built"));
        c.hello = hello;
    }

    /// What `axelstra bridge` connects with: the user's CLI on this machine, never presenting an agent credential
    /// (`AXELSTRA_AGENT_TOKEN` is not even read). The role then comes from the bridge process alone.
    pub fn connect_bridge(pipe: &str, token: &str) -> Result<Client> {
        Self::connect_with(pipe, token, Some("cli"), None)
    }

    /// The daemon on another machine, through `ssh` or `wsl.exe` running `axelstra bridge` there. The whole connect
    /// (the child starting, the bridge announcing itself, `hello`) must be done within `connect_timeout`, else the child
    /// is killed. What the child wrote to stderr goes into the errors; the child is killed when the client is dropped.
    pub fn connect_remote(spec: &crate::remote::RemoteSpec, connect_timeout: Duration) -> Result<Client> {
        let (program, args, env) = spec.command();
        let label = spec.to_string();
        let conn = spawn_child(&program, &args, &env, &label)?;
        let mut c = Client {
            rd: Conn::Child(conn.clone()),
            wr: Conn::Child(conn),
            next: 0,
            proto: 0,
            instance: 0,
            timeout: Some(connect_timeout),
            ceiling: ceiling(),
            broken: false,
            build: String::new(),
            security: 0,
            version: String::new(),
            commit: String::new(),
            built: String::new(),
            hello: Value::Null,
        };
        let started = std::time::Instant::now();
        let deadline = Deadline::start(&c.rd, connect_timeout).ok();
        let banner = crate::remote::scan_banner(&mut c.rd);
        let fired = deadline.map(Deadline::finish).unwrap_or(false);
        if let Err(e) = banner {
            let e = if fired {
                axelstra_core::coded!(
                    "host.no_answer",
                    "no answer within {timeout} (is `axelstra` installed there, and its daemon running?)",
                    timeout = format!("{connect_timeout:?}")
                )
            } else {
                e
            };
            return Err(c.explain(e.context(format!("could not reach {label}"))));
        }
        // what is left of the budget bounds `hello` (the bridge already did the daemon's)
        c.timeout = Some(connect_timeout.saturating_sub(started.elapsed()).max(Duration::from_secs(1)));
        let hello = c.call("hello", json!({"client": "cli"})).map_err(|e| e.context(format!("could not reach {label}")))?;
        c.adopt_hello(hello);
        c.timeout = None;
        Ok(c)
    }

    /// Pid of the `ssh`/`wsl.exe` child of a remote connection.
    pub fn child_id(&self) -> Option<u32> {
        match &self.rd {
            Conn::Child(c) => Some(recover(&c.shared.child).id()),
            _ => None,
        }
    }

    /// Ends the `ssh`/`wsl.exe` child of a remote connection now, so a thread blocked in `read` on a clone of it sees the
    /// end of the stream (the child otherwise lives until the last clone is dropped). No effect on a local connection.
    pub fn kill_child(&self) {
        if let Conn::Child(c) = &self.rd {
            let _ = recover(&c.shared.child).kill();
        }
    }

    /// The executable of the daemon behind a local connection: from the OS (the pipe's server process on Windows, the
    /// socket's peer on Linux and macOS), so it also works for a daemon from before `hello` said it; else the `exe` that
    /// `hello` reported. `None` for a remote connection or when neither is known.
    pub fn server_exe(&self) -> Option<std::path::PathBuf> {
        let from_os = match &self.rd {
            #[cfg(windows)]
            Conn::File(f) => crate::pipesec::server_exe(f),
            #[cfg(unix)]
            Conn::Unix(s) => peer_exe(s),
            Conn::Child(_) => return None,
        };
        from_os.or_else(|| self.hello["exe"].as_str().filter(|e| !e.is_empty()).map(std::path::PathBuf::from))
    }

    /// The `hello` result the daemon gave (for the bridge, which answers its own client's `hello` with it).
    pub fn hello_result(&self) -> &Value {
        &self.hello
    }

    /// Splits the connection into its two directions, for the bridge to copy bytes through.
    pub fn into_streams(self) -> (Box<dyn Read + Send>, Box<dyn Write + Send>) {
        #[cfg(windows)]
        let rd = PeekRead(self.rd);
        #[cfg(not(windows))]
        let rd = self.rd;
        (Box::new(rd), Box::new(self.wr))
    }

    /// Adds what a remote child said (stderr) and how it ended to an error that is not the daemon's own answer.
    fn explain(&self, e: anyhow::Error) -> anyhow::Error {
        if e.downcast_ref::<Remote>().is_some() {
            return e;
        }
        match &self.rd {
            Conn::Child(c) => match c.shared.diagnostics() {
                Some(d) => e.context(d),
                None => e,
            },
            _ => e,
        }
    }

    /// Independent handle for a second thread (e.g. one reads while one writes).
    pub fn try_clone(&self) -> Result<Client> {
        Ok(Client {
            rd: self.rd.try_clone()?,
            wr: self.wr.try_clone()?,
            next: 1 << 40,
            proto: self.proto,
            instance: self.instance,
            timeout: self.timeout,
            ceiling: self.ceiling,
            broken: self.broken,
            build: self.build.clone(),
            security: self.security,
            version: self.version.clone(),
            commit: self.commit.clone(),
            built: self.built.clone(),
            hello: self.hello.clone(),
        })
    }

    /// Sends a request and waits for its reply. The call ends by event: the reply, or the connection closing (the daemon
    /// or the `ssh`/`wsl.exe` child went away: `rpc.connection_closed`). No elapsed time fails it, except an explicit
    /// `timeout`, the cosmetic `policy_limit` of a method, or, for a lost handle, the high `ceiling` (`CallTimeout`, after
    /// which the connection is not reused). Streaming reads (`read`) are not bounded.
    pub fn call(&mut self, method: &str, params: Value) -> Result<Value> {
        if self.broken {
            // an earlier call ended this connection (closed or past the ceiling): it is not reused, the caller reconnects
            return Err(connection_closed(method));
        }
        self.next += 1;
        let id = self.next;
        let req = json!({"id": id, "method": method, "params": params});
        self.write_bounded(&encode(JSON, req.to_string().as_bytes())).map_err(|e| self.explain(anyhow::Error::new(NotSent(e))))?;
        let policy = self.timeout.or_else(|| policy_limit(method));
        let limit = policy.unwrap_or(self.ceiling);
        let deadline = Deadline::start(&self.rd, limit).ok();
        let res = self.reply(method, id);
        let fired = deadline.map(Deadline::finish).unwrap_or(false);
        match res {
            Err(e) if fired || is_timeout(&e) => {
                self.broken = true;
                let ceiling = policy.is_none();
                let msg = if ceiling {
                    format!("{method}: no reply within the safety ceiling of {limit:?}; the connection was dropped (AXELSTRA_CALL_CEILING_SECS changes it)")
                } else {
                    format!("{method}: the daemon did not answer within {limit:?}")
                };
                Err(self.explain(anyhow::Error::new(call_timeout(method, limit, msg, ceiling))))
            }
            Err(e) if is_closed(&e) => {
                self.broken = true;
                Err(self.explain(connection_closed(method)))
            }
            Err(e) => Err(self.explain(e)),
            r => r,
        }
    }

    /// Writes a whole frame. A write blocked on a full pipe (a daemon that stopped reading) ends with the connection or at
    /// the ceiling. A write that fails or is cut off may leave half a frame on the wire, so the connection is not used again.
    fn write_bounded(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        let deadline = Deadline::start(&self.wr, self.ceiling).ok();
        let mut r = self.wr.write_all(bytes);
        let fired = deadline.map(Deadline::finish).unwrap_or(false);
        if fired && r.is_err() {
            r = Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "the write to the daemon did not finish within the safety ceiling"));
        }
        if r.is_err() {
            self.broken = true;
        }
        r
    }

    fn reply(&mut self, method: &str, id: u64) -> Result<Value> {
        loop {
            match self.read()? {
                Frame::Response(v) if v["id"].as_u64() == Some(id) => {
                    if let Some(e) = v.get("error") {
                        let text = format!("{method}: {}", e.as_str().unwrap_or("error"));
                        let coded = axelstra_core::msg::Coded::from_wire(v["code"].as_str().unwrap_or(""), &v["params"], &text);
                        return Err(anyhow::Error::new(Remote(text, coded)));
                    }
                    return Ok(v["result"].clone());
                }
                _ => {} // output/events before our reply are not expected; skip
            }
        }
    }

    /// Sends a request without waiting for the reply; whoever reads this connection sees the response frame.
    pub fn send(&mut self, method: &str, params: Value) -> Result<()> {
        self.next += 1;
        let req = json!({"id": self.next, "method": method, "params": params});
        self.write_bounded(&encode(JSON, req.to_string().as_bytes()))?;
        Ok(())
    }

    pub fn send_input(&mut self, session: u32, data: &[u8]) -> Result<()> {
        self.write_bounded(&session_frame(INPUT, session, data))?;
        Ok(())
    }

    /// `read` for a connection another thread writes to at the same time (the app's event stream). On Windows a pending
    /// `ReadFile` on the synchronous pipe handle keeps a write on its duplicate waiting until a frame arrives, so a
    /// request sent on a quiet stream would not reach the daemon (and a deadline would cancel the read).
    /// This waits for data with `PeekNamedPipe`, like `PeekRead`, and reads only once a frame has started.
    pub fn read_shared(&mut self) -> Result<Frame> {
        #[cfg(windows)]
        if let Conn::File(f) = &self.rd {
            use std::os::windows::io::AsRawHandle;
            use windows_sys::Win32::System::Pipes::PeekNamedPipe;
            let mut nap = Duration::from_millis(1);
            loop {
                let mut avail = 0u32;
                let ok = unsafe {
                    PeekNamedPipe(f.as_raw_handle() as _, std::ptr::null_mut(), 0, std::ptr::null_mut(), &mut avail, std::ptr::null_mut())
                };
                if ok == 0 {
                    bail!("the daemon closed the connection");
                }
                if avail > 0 {
                    break;
                }
                // kept: same as PeekRead, no readiness event on a synchronous pipe handle
                std::thread::sleep(nap);
                nap = (nap * 2).min(Duration::from_millis(10));
            }
        }
        self.read()
    }

    pub fn read(&mut self) -> Result<Frame> {
        let mut len = [0u8; 4];
        self.rd.read_exact(&mut len)?;
        let len = u32::from_le_bytes(len) as usize;
        if len == 0 || len > MAX_FRAME {
            bail!("bad frame length {len}");
        }
        let mut kind = [0u8; 1];
        self.rd.read_exact(&mut kind)?;
        let kind = kind[0];
        let mut buf = vec![0u8; len - 1];
        self.rd.read_exact(&mut buf)?;
        Ok(match kind {
            OUTPUT => {
                let (session, d) = split_session(&buf).context("short output frame")?;
                Frame::Output { session, data: d.to_vec() }
            }
            EVENT => Frame::Event(serde_json::from_slice(&buf)?),
            JSON => Frame::Response(serde_json::from_slice(&buf)?),
            k => bail!("unknown frame kind {k}"),
        })
    }
}

/// A small pool of connections of one kind (the app's request/response calls). The daemon runs the calls of one connection
/// in order, and a caller holds its connection for the whole call, so with a single connection one slow reply (a git scan
/// of many worktrees) makes every later call wait behind it. With several, a slow call only occupies its own slot; the
/// first `fast_only` slots never take a slow method, so a quick call (`project.list`, `session.list`) always finds a free one.
pub struct Lanes<T> {
    slots: Vec<Mutex<Option<T>>>,
    claimed: Vec<std::sync::atomic::AtomicBool>,
    fast_only: usize,
    turn: std::sync::atomic::AtomicUsize,
}

/// A slot reserved by `Lanes::pick` for one call; it is free for the next caller when this drops.
pub struct Claim<'a, T> {
    lanes: &'a Lanes<T>,
    idx: usize,
    owned: bool,
}

impl<T> Lanes<T> {
    /// `total` slots (at least `fast_only` + 1), the first `fast_only` of them kept for quick methods.
    pub fn new(total: usize, fast_only: usize) -> Self {
        let total = total.max(fast_only + 1);
        Lanes {
            slots: (0..total).map(|_| Mutex::new(None)).collect(),
            claimed: (0..total).map(|_| std::sync::atomic::AtomicBool::new(false)).collect(),
            fast_only,
            turn: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    /// An idle slot for a call (`slow`: it may take long; those never use the quick-only slots). When every allowed slot
    /// is busy the caller queues on one of them, round robin, instead of failing.
    pub fn pick(&self, slow: bool) -> Claim<'_, T> {
        use std::sync::atomic::Ordering::SeqCst;
        let allowed = if slow { self.fast_only..self.slots.len() } else { 0..self.slots.len() };
        // a quick call tries the reserved slots first, so the others stay free for slow ones
        for i in allowed.clone() {
            if self.claimed[i].compare_exchange(false, true, SeqCst, SeqCst).is_ok() {
                return Claim { lanes: self, idx: i, owned: true };
            }
        }
        let n = allowed.len();
        let idx = allowed.start + self.turn.fetch_add(1, SeqCst) % n;
        Claim { lanes: self, idx, owned: false }
    }
}

impl<T> Claim<'_, T> {
    pub fn slot(&self) -> &Mutex<Option<T>> {
        &self.lanes.slots[self.idx]
    }
}

impl<T> Drop for Claim<'_, T> {
    fn drop(&mut self) {
        if self.owned {
            self.lanes.claimed[self.idx].store(false, std::sync::atomic::Ordering::SeqCst);
        }
    }
}
