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
        let said = String::from_utf8_lossy(&recover(&self.stderr)).trim().to_string();
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
pub struct Remote(pub String);

impl std::fmt::Display for Remote {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Remote {}

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
pub struct CallTimeout(pub String);

impl std::fmt::Display for CallTimeout {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for CallTimeout {}

/// How long `call` waits for the reply. Calls that run git, the network or an agent CLI get longer; none wait forever.
fn default_timeout(method: &str) -> Duration {
    match method {
        "git.push" | "git.pr" | "git.suggest" | "git.commit" | "worktree.create" | "worktree.from_pr" | "worktree.sweep"
        | "project.add" | "automation.run" | "orch.start" | "gh.list" | "agent.search" | "git.merge" | "git.checks" => {
            Duration::from_secs(600)
        }
        _ => Duration::from_secs(60),
    }
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
    /// when set, every `call` waits this long for its reply instead of the per-method default
    pub timeout: Option<Duration>,
    /// a call timed out: the stream may hold a late reply, so this connection must not be reused
    broken: bool,
    /// build id the daemon reported in `hello` (empty for a daemon from before it existed)
    pub build: String,
    /// `SECURITY_MIN` of the daemon (0 for a daemon from before it existed)
    pub security: u64,
    /// the whole `hello` result, for the bridge to hand on to its own client
    hello: Value,
}

/// How long one write to the daemon may block (a daemon that stopped reading with a full pipe).
const WRITE_LIMIT: Duration = Duration::from_secs(30);

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
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => return Err(e).with_context(|| format!("daemon not reachable at {pipe}")),
        }
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
            std::thread::sleep(nap);
            nap = (nap * 2).min(Duration::from_millis(10));
        }
    }
}

fn spawn_child(program: &str, args: &[String], env: &[(String, String)], label: &str) -> Result<ChildConn> {
    use std::process::{Command, Stdio};
    let mut cmd = Command::new(program);
    cmd.args(args).envs(env.iter().map(|(k, v)| (k, v))).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW: the app has no console, ssh/wsl must not flash one
        cmd.creation_flags(0x0800_0000);
    }
    let mut child = cmd.spawn().with_context(|| format!("could not start `{program}` for {label}"))?;
    let (stdin, stdout, stderr) = (child.stdin.take(), child.stdout.take(), child.stderr.take());
    let (Some(stdin), Some(stdout), Some(mut stderr)) = (stdin, stdout, stderr) else {
        let _ = child.kill();
        let _ = child.wait();
        bail!("could not attach to the pipes of `{program}`");
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
            broken: false,
            build: String::new(),
            security: 0,
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
            broken: false,
            build: String::new(),
            security: 0,
            hello: Value::Null,
        };
        let started = std::time::Instant::now();
        let deadline = Deadline::start(&c.rd, connect_timeout).ok();
        let banner = crate::remote::scan_banner(&mut c.rd);
        let fired = deadline.map(Deadline::finish).unwrap_or(false);
        if let Err(e) = banner {
            let e = if fired {
                anyhow::anyhow!("no answer within {connect_timeout:?} (is `axelstra` installed there, and its daemon running?)")
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
            broken: self.broken,
            build: self.build.clone(),
            security: self.security,
            hello: self.hello.clone(),
        })
    }

    /// Sends a request and waits for its reply, at most `timeout` (or the method's default): a hung daemon
    /// gives a `CallTimeout` error instead of freezing the caller. Streaming reads (`read`) are not bounded.
    pub fn call(&mut self, method: &str, params: Value) -> Result<Value> {
        if self.broken {
            return Err(anyhow::Error::new(CallTimeout(format!("{method}: the connection to the daemon timed out earlier; reconnect"))));
        }
        self.next += 1;
        let id = self.next;
        let req = json!({"id": id, "method": method, "params": params});
        self.write_bounded(&encode(JSON, req.to_string().as_bytes())).map_err(|e| self.explain(anyhow::Error::new(NotSent(e))))?;
        let limit = self.timeout.unwrap_or_else(|| default_timeout(method));
        let deadline = Deadline::start(&self.rd, limit).ok();
        let res = self.reply(method, id);
        let fired = deadline.map(Deadline::finish).unwrap_or(false);
        match res {
            Err(e) if fired || is_timeout(&e) => {
                self.broken = true;
                let msg = format!("{method}: the daemon did not answer within {limit:?} (it may be hung); reconnect or restart it");
                Err(self.explain(anyhow::Error::new(CallTimeout(msg))))
            }
            Err(e) => Err(self.explain(e)),
            r => r,
        }
    }

    /// Writes a whole frame within `WRITE_LIMIT`. A write that fails or is cut off may leave half a frame on the wire, so
    /// the connection is not used again.
    fn write_bounded(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        let deadline = Deadline::start(&self.wr, WRITE_LIMIT).ok();
        let mut r = self.wr.write_all(bytes);
        let fired = deadline.map(Deadline::finish).unwrap_or(false);
        if fired && r.is_err() {
            r = Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "the write to the daemon did not finish in time"));
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
                        return Err(anyhow::Error::new(Remote(format!("{method}: {}", e.as_str().unwrap_or("error")))));
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
