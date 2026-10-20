//! Runs orchestrator agents as PTY sessions inside the daemon, so the UI can
//! attach to them, type into them and watch them live.

use crate::client::Client;
use crate::proto;
use anyhow::Result;
use axelstra_core::runner::{AgentSpawn, Handle, Runner};
use serde_json::json;
use std::sync::{Arc, Mutex};

pub struct DaemonRunner {
    client: Arc<Mutex<Client>>,
}

impl DaemonRunner {
    pub fn connect(pipe: &str, token: &str) -> Result<DaemonRunner> {
        Ok(DaemonRunner { client: Arc::new(Mutex::new(Client::connect(pipe, token)?)) })
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
