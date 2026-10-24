//! Blocking client (named pipe opened as a plain file on Windows, Unix socket elsewhere),
//! so callers need no async runtime.

use crate::proto::*;
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
#[cfg(windows)]
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
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
}

impl Conn {
    fn try_clone(&self) -> std::io::Result<Conn> {
        Ok(match self {
            #[cfg(windows)]
            Conn::File(f) => Conn::File(f.try_clone()?),
            #[cfg(unix)]
            Conn::Unix(s) => Conn::Unix(s.try_clone()?),
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
        }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            #[cfg(windows)]
            Conn::File(f) => f.flush(),
            #[cfg(unix)]
            Conn::Unix(s) => s.flush(),
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
        | "project.add" | "automation.run" | "orch.start" | "gh.list" | "gh.comments" | "gh.send_comment" | "git.suggest_pr"
        | "agent.search" | "git.merge" | "git.checks" => Duration::from_secs(600),
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

/// One watchdog thread for every deadline of the process: calls register `(handle, time)` and the thread cancels the
/// pending I/O of the ones that run out. (It used to be one OS thread per call.)
#[cfg(windows)]
mod watchdog {
    use std::sync::{Condvar, Mutex, Once};
    use std::time::{Duration, Instant};

    struct Entry {
        id: u64,
        at: Instant,
        handle: isize,
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

    pub fn arm(handle: isize, limit: Duration) -> u64 {
        START.call_once(|| {
            std::thread::Builder::new().name("client-watchdog".into()).spawn(run).expect("watchdog thread");
        });
        let mut st = lock();
        st.next += 1;
        let id = st.next;
        st.entries.push(Entry { id, at: Instant::now() + limit, handle });
        drop(st);
        WAKE.notify_all();
        id
    }

    /// Returns true when the deadline had already fired (the I/O was cancelled).
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
            // cancelling happens under the lock, so `disarm` cannot return while a cancel is about to land on a later
            // call's I/O on the same handle
            st.entries.retain(|e| {
                if e.at <= now {
                    unsafe { windows_sys::Win32::System::IO::CancelIoEx(e.handle as _, std::ptr::null()) };
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
struct Deadline {
    #[cfg(windows)]
    id: u64,
    #[cfg(unix)]
    sock: std::os::unix::net::UnixStream,
}

#[cfg(windows)]
impl Deadline {
    fn start(conn: &Conn, limit: Duration) -> std::io::Result<Deadline> {
        use std::os::windows::io::AsRawHandle;
        let Conn::File(f) = conn;
        Ok(Deadline { id: watchdog::arm(f.as_raw_handle() as isize, limit) })
    }

    fn finish(self) {
        watchdog::disarm(self.id);
    }
}

#[cfg(unix)]
impl Deadline {
    fn start(conn: &Conn, limit: Duration) -> std::io::Result<Deadline> {
        let Conn::Unix(s) = conn;
        let sock = s.try_clone()?;
        sock.set_read_timeout(Some(limit))?;
        sock.set_write_timeout(Some(limit))?;
        Ok(Deadline { sock })
    }

    fn finish(self) {
        let _ = self.sock.set_read_timeout(None);
        let _ = self.sock.set_write_timeout(None);
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
        };
        let hello = c.call("hello", json!({"token": token, "client": client, "agent_token": agent_token}))?;
        c.proto = hello["proto"].as_u64().unwrap_or(1);
        c.instance = hello["instance"].as_u64().unwrap_or(0);
        c.build = hello["build"].as_str().unwrap_or_default().to_string();
        c.security = hello["security"].as_u64().unwrap_or(0);
        Ok(c)
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
        self.write_bounded(&encode(JSON, req.to_string().as_bytes())).map_err(|e| anyhow::Error::new(NotSent(e)))?;
        let limit = self.timeout.unwrap_or_else(|| default_timeout(method));
        let deadline = Deadline::start(&self.rd, limit).ok();
        let res = self.reply(method, id);
        if let Some(d) = deadline {
            d.finish();
        }
        match res {
            Err(e) if is_timeout(&e) => {
                self.broken = true;
                let msg = format!("{method}: the daemon did not answer within {limit:?} (it may be hung); reconnect or restart it");
                Err(anyhow::Error::new(CallTimeout(msg)))
            }
            r => r,
        }
    }

    /// Writes a whole frame within `WRITE_LIMIT`. A write that fails or is cut off may leave half a frame on the wire, so
    /// the connection is not used again.
    fn write_bounded(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        let deadline = Deadline::start(&self.wr, WRITE_LIMIT).ok();
        let r = self.wr.write_all(bytes);
        if let Some(d) = deadline {
            d.finish();
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
