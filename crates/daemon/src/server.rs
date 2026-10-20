use crate::proto::*;
use anyhow::Result;
use axelstra_core::pty::{Session, Spawn};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::RecvTimeoutError;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};
use tokio::sync::mpsc;

struct Registry {
    next: AtomicU32,
    sessions: Mutex<HashMap<u32, Arc<Session>>>,
    labels: Mutex<HashMap<u32, String>>,
    /// free-form UI data per session (project, worktree...), returned by session.list
    metas: Mutex<HashMap<u32, Value>>,
    store: std::path::PathBuf,
}

impl Registry {
    fn get(&self, id: u32) -> Option<Arc<Session>> {
        self.sessions.lock().unwrap().get(&id).cloned()
    }
}

#[derive(Deserialize)]
struct Req {
    id: u64,
    method: String,
    #[serde(default)]
    params: Value,
}

/// Serves until the process dies. Fails if another daemon owns the pipe.
pub async fn serve(pipe: String, token: String) -> Result<()> {
    serve_with_store(pipe, token, crate::projects::store_path()).await
}

/// Same as `serve`, with an explicit projects file (tests must not touch the user's).
pub async fn serve_with_store(pipe: String, token: String, store: std::path::PathBuf) -> Result<()> {
    let reg = Arc::new(Registry {
        next: AtomicU32::new(0),
        sessions: Default::default(),
        labels: Default::default(),
        metas: Default::default(),
        store,
    });
    let mut server = ServerOptions::new()
        .first_pipe_instance(true)
        .reject_remote_clients(true)
        .create(&pipe)?;
    loop {
        server.connect().await?;
        let conn = server;
        server = ServerOptions::new().reject_remote_clients(true).create(&pipe)?;
        tokio::spawn(handle(conn, reg.clone(), token.clone()));
    }
}

async fn read_frame(rd: &mut (impl AsyncReadExt + Unpin)) -> std::io::Result<(u8, Vec<u8>)> {
    let mut len = [0u8; 4];
    rd.read_exact(&mut len).await?;
    let len = u32::from_le_bytes(len) as usize;
    if len == 0 || len > MAX_FRAME {
        return Err(std::io::ErrorKind::InvalidData.into());
    }
    let mut buf = vec![0u8; len];
    rd.read_exact(&mut buf).await?;
    let kind = buf[0];
    buf.remove(0);
    Ok((kind, buf))
}

async fn handle(pipe: NamedPipeServer, reg: Arc<Registry>, token: String) {
    let (mut rd, mut wr) = tokio::io::split(pipe);
    let (tx, mut rx) = mpsc::unbounded_channel::<Vec<u8>>();
    tokio::spawn(async move {
        while let Some(f) = rx.recv().await {
            if wr.write_all(&f).await.is_err() {
                break;
            }
        }
    });

    let mut authed = false;
    let mut subs: HashMap<u32, Arc<AtomicBool>> = HashMap::new();

    while let Ok((kind, payload)) = read_frame(&mut rd).await {
        match kind {
            JSON => {
                let Ok(req) = serde_json::from_slice::<Req>(&payload) else { break };
                if !authed && req.method != "hello" {
                    break;
                }
                let mut after: Vec<Box<dyn FnOnce() + Send>> = Vec::new();
                let res = dispatch(&req, &reg, &tx, &mut subs, &mut authed, &token, &mut after);
                let body = match res {
                    Ok(v) => json!({"id": req.id, "result": v}),
                    Err(e) => json!({"id": req.id, "error": format!("{e:#}")}),
                };
                let _ = tx.send(encode(JSON, body.to_string().as_bytes()));
                // deferred work (stream start) runs only after the reply is queued
                for f in after {
                    f();
                }
                if req.method == "hello" && !authed {
                    break;
                }
            }
            INPUT if authed => {
                if let Some((id, data)) = split_session(&payload) {
                    if let Some(s) = reg.get(id) {
                        let _ = s.write(data);
                    }
                }
            }
            _ => break,
        }
    }
    // client gone: stop forwarders, sessions keep running
    for stop in subs.values() {
        stop.store(true, Ordering::SeqCst);
    }
}

fn dispatch(
    req: &Req,
    reg: &Arc<Registry>,
    tx: &mpsc::UnboundedSender<Vec<u8>>,
    subs: &mut HashMap<u32, Arc<AtomicBool>>,
    authed: &mut bool,
    token: &str,
    after: &mut Vec<Box<dyn FnOnce() + Send>>,
) -> Result<Value> {
    let p = &req.params;
    let sid = || -> Result<u32> {
        Ok(p.get("id").and_then(Value::as_u64).ok_or_else(|| anyhow::anyhow!("missing id"))? as u32)
    };
    let session = || -> Result<Arc<Session>> {
        reg.get(sid()?).ok_or_else(|| anyhow::anyhow!("no such session"))
    };
    match req.method.as_str() {
        "hello" => {
            if p.get("token").and_then(Value::as_str) != Some(token) {
                anyhow::bail!("bad token");
            }
            *authed = true;
            Ok(json!({"proto": 1}))
        }
        "session.spawn" => {
            let s = Spawn {
                program: p["program"].as_str().ok_or_else(|| anyhow::anyhow!("missing program"))?.into(),
                args: p["args"]
                    .as_array()
                    .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
                    .unwrap_or_default(),
                cwd: p["cwd"].as_str().map(Into::into),
                env: p["env"]
                    .as_object()
                    .map(|o| o.iter().filter_map(|(k, v)| v.as_str().map(|v| (k.clone(), v.to_string()))).collect())
                    .unwrap_or_default(),
                cols: p["cols"].as_u64().unwrap_or(120) as u16,
                rows: p["rows"].as_u64().unwrap_or(30) as u16,
            };
            let sess = Arc::new(Session::spawn(&s)?);
            let id = reg.next.fetch_add(1, Ordering::SeqCst) + 1;
            reg.sessions.lock().unwrap().insert(id, sess);
            if let Some(l) = p["label"].as_str() {
                reg.labels.lock().unwrap().insert(id, l.to_string());
            }
            if !p["meta"].is_null() {
                reg.metas.lock().unwrap().insert(id, p["meta"].clone());
            }
            Ok(json!({"id": id}))
        }
        "session.list" => {
            let l = reg.sessions.lock().unwrap();
            let labels = reg.labels.lock().unwrap();
            let metas = reg.metas.lock().unwrap();
            let mut v: Vec<Value> = l
                .iter()
                .map(|(id, s)| {
                    json!({"id": id, "running": s.try_wait().is_none(), "label": labels.get(id), "meta": metas.get(id)})
                })
                .collect();
            v.sort_by_key(|x| x["id"].as_u64());
            Ok(Value::Array(v))
        }
        "session.subscribe" => {
            let id = sid()?;
            let s = session()?;
            if let Some(old) = subs.remove(&id) {
                old.store(true, Ordering::SeqCst);
            }
            let stop = Arc::new(AtomicBool::new(false));
            subs.insert(id, stop.clone());
            let (snap, rx) = s.subscribe();
            let tx = tx.clone();
            // plain thread: the PTY side is blocking std channels
            after.push(Box::new(move || {
                std::thread::spawn(move || {
                if !snap.is_empty() {
                    let _ = tx.send(session_frame(OUTPUT, id, &snap));
                }
                loop {
                    if stop.load(Ordering::SeqCst) {
                        return;
                    }
                    match rx.recv_timeout(Duration::from_millis(200)) {
                        Ok(d) => {
                            if tx.send(session_frame(OUTPUT, id, &d)).is_err() {
                                return;
                            }
                        }
                        Err(RecvTimeoutError::Timeout) => {
                            // ConPTY may keep the output pipe open after the process is gone
                            if s.try_wait().is_some() {
                                while let Ok(d) = rx.recv_timeout(Duration::from_millis(150)) {
                                    let _ = tx.send(session_frame(OUTPUT, id, &d));
                                }
                                break;
                            }
                        }
                        Err(RecvTimeoutError::Disconnected) => break,
                    }
                }
                let mut code = None;
                for _ in 0..40 {
                    code = s.try_wait();
                    if code.is_some() {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
                let ev = json!({"event": "session.exit", "id": id, "code": code});
                let _ = tx.send(encode(EVENT, ev.to_string().as_bytes()));
                });
            }));
            Ok(json!({}))
        }
        "session.unsubscribe" => {
            if let Some(stop) = subs.remove(&sid()?) {
                stop.store(true, Ordering::SeqCst);
            }
            Ok(json!({}))
        }
        "session.resize" => {
            session()?.resize(p["cols"].as_u64().unwrap_or(120) as u16, p["rows"].as_u64().unwrap_or(30) as u16)?;
            Ok(json!({}))
        }
        "session.status" => {
            let code = session()?.try_wait();
            Ok(json!({"running": code.is_none(), "code": code}))
        }
        "session.remove" => {
            let id = sid()?;
            if let Some(stop) = subs.remove(&id) {
                stop.store(true, Ordering::SeqCst);
            }
            if let Some(s) = reg.sessions.lock().unwrap().remove(&id) {
                s.kill();
            }
            reg.labels.lock().unwrap().remove(&id);
            reg.metas.lock().unwrap().remove(&id);
            Ok(json!({}))
        }
        "session.kill" => {
            session()?.kill();
            Ok(json!({}))
        }
        "session.usage" => {
            let (mem, cpu) = session()?.job_usage();
            Ok(json!({"peak_mem_bytes": mem, "cpu_100ns": cpu}))
        }
        "project.list" => {
            let ps = crate::projects::load(&reg.store);
            let v: Vec<Value> = ps
                .iter()
                .map(|pr| {
                    let repo = std::path::Path::new(&pr.path);
                    let wts = axelstra_core::git::worktree_list(repo).unwrap_or_default();
                    let ident = axelstra_core::git::identity(repo);
                    json!({"id": pr.id, "name": ident.display_name, "path": pr.path, "worktrees": wts,
                           "slug": ident.slug, "icon_url": ident.icon_url, "remote": ident.remote})
                })
                .collect();
            Ok(Value::Array(v))
        }
        "git.changes" => {
            let wt = crate::projects::known_worktree(&reg.store, p["worktree"].as_str().unwrap_or_default())?;
            let base = p["base"].as_str();
            Ok(serde_json::to_value(tokio::task::block_in_place(|| crate::changes::changes(&wt, base))?)?)
        }
        "git.diff" => {
            let wt = crate::projects::known_worktree(&reg.store, p["worktree"].as_str().unwrap_or_default())?;
            let path = p["path"].as_str().ok_or_else(|| anyhow::anyhow!("missing path"))?;
            let base = p["base"].as_str();
            let text = tokio::task::block_in_place(|| crate::changes::diff(&wt, path, base))?;
            Ok(json!({"diff": text}))
        }
        "git.commit" | "git.push" | "git.pr" => {
            let wt = crate::projects::known_worktree(&reg.store, p["worktree"].as_str().unwrap_or_default())?;
            let s = |k: &str| p[k].as_str().unwrap_or_default().to_string();
            tokio::task::block_in_place(|| -> Result<Value> {
                use crate::publish;
                Ok(match req.method.as_str() {
                    "git.commit" => serde_json::to_value(publish::commit(&wt, &s("message"))?)?,
                    "git.push" => json!({"output": publish::push(&wt)?}),
                    _ => json!({"url": publish::pull_request(&wt, &s("title"), &s("body"), p["base"].as_str())?}),
                })
            })
        }
        "fs.list" | "fs.read" | "fs.stat" | "fs.write" => {
            let wt = crate::projects::known_worktree(&reg.store, p["worktree"].as_str().unwrap_or_default())?;
            let path = p["path"].as_str().unwrap_or_default();
            tokio::task::block_in_place(|| -> Result<Value> {
                use crate::fsops;
                Ok(match req.method.as_str() {
                    "fs.list" => json!({"entries": fsops::list(&wt, path)?}),
                    "fs.read" => serde_json::to_value(fsops::read(&wt, path)?)?,
                    "fs.stat" => json!({"hash": fsops::stat_hash(&wt, path)?}),
                    _ => {
                        let text = p["text"].as_str().ok_or_else(|| anyhow::anyhow!("missing text"))?;
                        let eol = p["eol"].as_str().unwrap_or("lf");
                        let bom = p["bom"].as_bool().unwrap_or(false);
                        match fsops::write(&wt, path, text, eol, bom, p["base_hash"].as_str())? {
                            fsops::Saved::Ok { hash } => json!({"hash": hash}),
                            fsops::Saved::Conflict { disk_hash } => json!({"conflict": true, "disk_hash": disk_hash}),
                        }
                    }
                })
            })
        }
        "project.add" => {
            let path = p["path"].as_str().ok_or_else(|| anyhow::anyhow!("missing path"))?;
            Ok(serde_json::to_value(crate::projects::add(&reg.store, path)?)?)
        }
        "project.remove" => {
            crate::projects::remove(&reg.store, p["id"].as_str().unwrap_or_default())?;
            Ok(json!({}))
        }
        "worktree.create" => {
            let pr = crate::projects::find(&reg.store, p["project"].as_str().unwrap_or_default())?;
            let name = p["name"].as_str().ok_or_else(|| anyhow::anyhow!("missing name"))?;
            Ok(serde_json::to_value(tokio::task::block_in_place(|| crate::projects::worktree_create(&pr, name))?)?)
        }
        "worktree.remove" => {
            let pr = crate::projects::find(&reg.store, p["project"].as_str().unwrap_or_default())?;
            let path = p["path"].as_str().ok_or_else(|| anyhow::anyhow!("missing path"))?;
            tokio::task::block_in_place(|| crate::projects::worktree_remove(&pr, path))?;
            Ok(json!({}))
        }
        "ping" => Ok(json!({})),
        m => anyhow::bail!("unknown method {m}"),
    }
}
