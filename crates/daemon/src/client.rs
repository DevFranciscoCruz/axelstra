//! Blocking client. Opens the named pipe as a plain file, so callers need no async runtime.

use crate::proto::*;
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::time::Duration;

pub enum Frame {
    Output { session: u32, data: Vec<u8> },
    Event(Value),
    Response(Value),
}

pub struct Client {
    rd: File,
    wr: File,
    next: u64,
}

impl Client {
    pub fn connect(pipe: &str, token: &str) -> Result<Client> {
        let mut tries = 0;
        let f = loop {
            match OpenOptions::new().read(true).write(true).open(pipe) {
                Ok(f) => break f,
                // ERROR_PIPE_BUSY: the daemon is between instances
                Err(e) if e.raw_os_error() == Some(231) && tries < 20 => {
                    tries += 1;
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(e) => return Err(e).with_context(|| format!("daemon not reachable at {pipe}")),
            }
        };
        let mut c = Client { rd: f.try_clone()?, wr: f, next: 0 };
        c.call("hello", json!({"token": token}))?;
        Ok(c)
    }

    /// Independent handle for a second thread (e.g. one reads while one writes).
    pub fn try_clone(&self) -> Result<Client> {
        Ok(Client { rd: self.rd.try_clone()?, wr: self.wr.try_clone()?, next: 1 << 40 })
    }

    pub fn call(&mut self, method: &str, params: Value) -> Result<Value> {
        self.next += 1;
        let id = self.next;
        let req = json!({"id": id, "method": method, "params": params});
        self.wr.write_all(&encode(JSON, req.to_string().as_bytes()))?;
        loop {
            match self.read()? {
                Frame::Response(v) if v["id"].as_u64() == Some(id) => {
                    if let Some(e) = v.get("error") {
                        bail!("{method}: {}", e.as_str().unwrap_or("error"));
                    }
                    return Ok(v["result"].clone());
                }
                _ => {} // output/events before our reply are not expected; skip
            }
        }
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
