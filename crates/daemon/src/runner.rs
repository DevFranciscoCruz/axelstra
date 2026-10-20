//! Runs orchestrator agents as PTY sessions inside the daemon, so the UI can
//! attach to them, type into them and watch them live.

use crate::client::{Client, Frame};
use crate::proto;
use anyhow::Result;
use axelstra_core::runner::{AgentSpawn, Handle, Runner};
use serde_json::json;
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub struct DaemonRunner {
    client: Arc<Mutex<Client>>,
    pipe: String,
    token: String,
}

impl DaemonRunner {
    pub fn connect(pipe: &str, token: &str) -> Result<DaemonRunner> {
        Ok(DaemonRunner {
            client: Arc::new(Mutex::new(Client::connect(pipe, token)?)),
            pipe: pipe.to_string(),
            token: token.to_string(),
        })
    }

    pub fn connect_default() -> Result<DaemonRunner> {
        Self::connect(&proto::pipe_name(), &proto::load_or_create_token()?)
    }
}

struct SessionHandle {
    client: Arc<Mutex<Client>>,
    id: u32,
}

impl Runner for DaemonRunner {
    fn spawn(&self, s: &AgentSpawn) -> Result<Box<dyn Handle>> {
        let env: serde_json::Map<String, serde_json::Value> =
            s.env.iter().map(|(k, v)| (k.clone(), json!(v))).collect();
        let r = self.client.lock().unwrap().call(
            "session.spawn",
            json!({
                "program": s.program, "args": s.args, "cwd": s.cwd,
                "env": env, "label": s.label, "cols": 120, "rows": 32,
            }),
        )?;
        let id = r["id"].as_u64().ok_or_else(|| anyhow::anyhow!("daemon returned no id"))? as u32;
        accept_trust_prompt(self.pipe.clone(), self.token.clone(), id);
        Ok(Box::new(SessionHandle { client: self.client.clone(), id }))
    }

    fn interactive(&self) -> bool {
        true
    }
}

impl Handle for SessionHandle {
    fn try_wait(&mut self) -> Result<Option<String>> {
        let r = self.client.lock().unwrap().call("session.status", json!({"id": self.id}))?;
        Ok(if r["running"] == true { None } else { Some(format!("exited ({})", r["code"])) })
    }

    fn kill(&mut self) {
        let _ = self.client.lock().unwrap().call("session.kill", json!({"id": self.id}));
    }
}

impl Drop for SessionHandle {
    fn drop(&mut self) {
        let _ = self.client.lock().unwrap().call("session.remove", json!({"id": self.id}));
    }
}

/// Terminal text without escape sequences. Cursor-forward (`ESC[nC`) becomes a
/// space, because TUIs use it instead of literal spaces between words.
pub fn plain(bytes: &[u8]) -> String {
    let mut out: Vec<u8> = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == 0x1b && i + 1 < bytes.len() {
            match bytes[i + 1] {
                b'[' => {
                    i += 2;
                    while i < bytes.len() && !(0x40..=0x7e).contains(&bytes[i]) {
                        i += 1;
                    }
                    if i < bytes.len() && bytes[i] == b'C' {
                        out.push(b' ');
                    }
                    i += 1;
                }
                b']' => {
                    i += 2;
                    while i < bytes.len() && bytes[i] != 0x07 && !(bytes[i] == 0x1b && bytes.get(i + 1) == Some(&b'\\')) {
                        i += 1;
                    }
                    i += if bytes.get(i) == Some(&0x07) { 1 } else { 2 };
                }
                _ => i += 2,
            }
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Claude Code asks "Is this a project you trust?" in every new folder and
/// preselects "No, exit". Our worktrees are created by us, so answer Yes once.
/// Runs on its own connection; ends when the prompt is answered or the session exits.
fn accept_trust_prompt(pipe: String, token: String, id: u32) {
    std::thread::spawn(move || {
        let Ok(mut c) = Client::connect(&pipe, &token) else { return };
        if c.call("session.subscribe", json!({"id": id})).is_err() {
            return;
        }
        let mut seen = Vec::new();
        while let Ok(f) = c.read() {
            match f {
                Frame::Output { data, .. } => {
                    seen.extend(data);
                    if seen.len() > 64 * 1024 {
                        seen.drain(..32 * 1024);
                    }
                    if plain(&seen).contains("trust this folder") {
                        // keys sent before the menu is interactive get lost, and Enter
                        // would then pick the default "No, exit"
                        std::thread::sleep(Duration::from_millis(1500));
                        let _ = c.send_input(id, b"\x1b[B"); // Down: "Yes, I trust this folder"
                        std::thread::sleep(Duration::from_millis(400));
                        let _ = c.send_input(id, b"\r");
                        return;
                    }
                }
                Frame::Event(e) if e["event"] == "session.exit" => return,
                _ => {}
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::plain;

    #[test]
    fn plain_turns_cursor_forward_into_spaces() {
        let raw = b"\x1b[8;2HQuick\x1b[1Csafety\x1b[1Ccheck:\x1b[1CIs\x1b[1Cthis\x1b[1Cproject\x1b[1Cyou\x1b[1Ctrust?\x1b[15;2HNo";
        let t = plain(raw);
        assert!(t.contains("Quick safety check: Is this project you trust?"), "{t}");
    }

    #[test]
    fn plain_strips_osc_and_csi() {
        let raw = b"\x1b]0;title\x07\x1b[38;2;1;2;3mhello\x1b[m world";
        assert_eq!(plain(raw), "hello world");
    }
}
