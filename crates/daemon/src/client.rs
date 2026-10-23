//! Blocking client (named pipe opened as a plain file on Windows, Unix socket elsewhere),
//! so callers need no async runtime.

use crate::proto::*;
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
#[cfg(windows)]
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
#[cfg(windows)]
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

pub struct Client {
    rd: Conn,
    wr: Conn,
    next: u64,
    /// protocol version the daemon reported in `hello`
    pub proto: u64,
    /// which daemon run answered `hello`; a different value on reconnect means the daemon restarted
    pub instance: u64,
}

#[cfg(windows)]
fn open(pipe: &str) -> Result<Conn> {
    let mut tries = 0;
    loop {
        match OpenOptions::new().read(true).write(true).open(pipe) {
            Ok(f) => return Ok(Conn::File(f)),
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
    let s = std::os::unix::net::UnixStream::connect(path)
        .with_context(|| format!("daemon not reachable at {path}"))?;
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

    fn connect_as(pipe: &str, token: &str, client: Option<&str>) -> Result<Client> {
        let f = open(pipe)?;
        let mut c = Client { rd: f.try_clone()?, wr: f, next: 0, proto: 0, instance: 0 };
        let hello = c.call("hello", json!({"token": token, "client": client}))?;
        c.proto = hello["proto"].as_u64().unwrap_or(1);
        c.instance = hello["instance"].as_u64().unwrap_or(0);
        Ok(c)
    }

    /// Independent handle for a second thread (e.g. one reads while one writes).
    pub fn try_clone(&self) -> Result<Client> {
        Ok(Client { rd: self.rd.try_clone()?, wr: self.wr.try_clone()?, next: 1 << 40, proto: self.proto, instance: self.instance })
    }

    pub fn call(&mut self, method: &str, params: Value) -> Result<Value> {
        self.next += 1;
        let id = self.next;
        let req = json!({"id": id, "method": method, "params": params});
        self.wr.write_all(&encode(JSON, req.to_string().as_bytes())).map_err(|e| anyhow::Error::new(NotSent(e)))?;
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
        self.wr.write_all(&encode(JSON, req.to_string().as_bytes()))?;
        Ok(())
    }

    pub fn send_input(&mut self, session: u32, data: &[u8]) -> Result<()> {
        self.wr.write_all(&session_frame(INPUT, session, data))?;
        Ok(())
    }

    pub fn read(&mut self) -> Result<Frame> {
        let mut len = [0u8; 4];
        self.rd.read_exact(&mut len)?;
        let len = u32::from_le_bytes(len) as usize;
        if len == 0 || len > MAX_FRAME {
            bail!("bad frame length {len}");
        }
        let mut buf = vec![0u8; len];
        self.rd.read_exact(&mut buf)?;
        let kind = buf.remove(0);
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
