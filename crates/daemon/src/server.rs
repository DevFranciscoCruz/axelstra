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
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::mpsc;

struct Registry {
    next: AtomicU32,
    sessions: Mutex<HashMap<u32, Arc<Session>>>,
    labels: Mutex<HashMap<u32, String>>,
    /// free-form UI data per session (project, worktree...), returned by session.list
    metas: Mutex<HashMap<u32, Value>>,
    store: std::path::PathBuf,
    /// where background orchestration runs connect back to this daemon
    pipe: String,
    token: String,
    /// latest orchestration run per project id
    runs: Mutex<HashMap<String, Value>>,
    /// serializes read-modify-write of automations.json (UI saves vs the scheduler)
    auto_lock: Mutex<()>,
}

impl Registry {
    fn get(&self, id: u32) -> Option<Arc<Session>> {
        self.sessions.lock().unwrap().get(&id).cloned()
    }

    /// Starts a session and records its label and UI metadata.
    fn spawn(&self, s: &Spawn, label: Option<&str>, meta: Option<Value>) -> Result<u32> {
        let sess = Arc::new(Session::spawn(s)?);
        let id = self.next.fetch_add(1, Ordering::SeqCst) + 1;
        self.sessions.lock().unwrap().insert(id, sess);
        if let Some(l) = label {
            self.labels.lock().unwrap().insert(id, l.to_string());
        }
        if let Some(m) = meta {
            self.metas.lock().unwrap().insert(id, m);
        }
        Ok(id)
    }
}

/// Wakes every 30 s and starts the automations that are due. Missed runs are not replayed.
fn spawn_scheduler(reg: Arc<Registry>) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(30));
        loop {
            tick.tick().await;
            let r = reg.clone();
            let _ = tokio::task::spawn_blocking(move || {
                let _g = r.auto_lock.lock().unwrap();
                let path = crate::automations::store_path(&r.store);
                let list = crate::automations::load(&path);
                for id in crate::automations::due_now(&list, crate::automations::now_ts()) {
                    if let Err(e) = run_automation(&r, &path, &id) {
                        eprintln!("automation {id} failed: {e:#}");
                    }
                }
            })
            .await;
        }
    });
}

/// Fresh worktree + agent session with the automation's prompt. Records the start first so a failure never loops.
/// The caller holds `auto_lock`.
fn run_automation(reg: &Registry, path: &std::path::Path, id: &str) -> Result<Value> {
    let mut list = crate::automations::load(path);
    let a = list.iter_mut().find(|a| a.id == id).ok_or_else(|| anyhow::anyhow!("unknown automation {id}"))?;
    a.last_run = crate::automations::now_ts();
    let a = a.clone();
    crate::automations::save_all(path, &list)?;

    let pr = crate::projects::find(&reg.store, &a.project)?;
    let stamp = chrono::Local::now().format("%m%d-%H%M").to_string();
    let wt = crate::projects::worktree_create(&pr, &format!("auto-{}-{stamp}", a.name))?;
    // AXELSTRA_AGENT_CLI swaps the agent binary (a wrapper script, or a harmless stand-in in tests)
    let cli = std::env::var("AXELSTRA_AGENT_CLI").unwrap_or_else(|_| a.agent.clone());
    let (program, args) = crate::automations::agent_command(&cli, &a.prompt);
    let spawn = Spawn { program, args, cwd: Some(wt.path.clone().into()), env: vec![], cols: 120, rows: 30 };
    let meta = json!({"project": pr.id, "worktree": wt.path, "kind": "agent", "automation": a.id});
    let sid = reg.spawn(&spawn, Some(&a.name), Some(meta))?;
    Ok(json!({"session": sid, "worktree": wt.path}))
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
        pipe: pipe.clone(),
        token: token.clone(),
        runs: Default::default(),
        auto_lock: Default::default(),
    });
    spawn_scheduler(reg.clone());
    accept_loop(&pipe, reg, token).await
}

#[cfg(windows)]
async fn accept_loop(pipe: &str, reg: Arc<Registry>, token: String) -> Result<()> {
    use tokio::net::windows::named_pipe::ServerOptions;
    let mut server = ServerOptions::new().first_pipe_instance(true).reject_remote_clients(true).create(pipe)?;
    loop {
        server.connect().await?;
        let conn = server;
        server = ServerOptions::new().reject_remote_clients(true).create(pipe)?;
        tokio::spawn(handle(conn, reg.clone(), token.clone()));
    }
}

#[cfg(unix)]
async fn accept_loop(path: &str, reg: Arc<Registry>, token: String) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    if let Some(dir) = std::path::Path::new(path).parent() {
        std::fs::create_dir_all(dir)?;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    // a live daemon owns the socket; a leftover file from a crashed one is replaced
    if std::path::Path::new(path).exists() {
        if std::os::unix::net::UnixStream::connect(path).is_ok() {
            anyhow::bail!("another daemon is already listening on {path}");
        }
        std::fs::remove_file(path)?;
    }
    let listener = tokio::net::UnixListener::bind(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    loop {
        let (conn, _) = listener.accept().await?;
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

async fn handle(pipe: impl AsyncRead + AsyncWrite + Send + 'static, reg: Arc<Registry>, token: String) {
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
            let meta = if p["meta"].is_null() { None } else { Some(p["meta"].clone()) };
            let id = reg.spawn(&s, p["label"].as_str(), meta)?;
            Ok(json!({"id": id}))
        }
        "session.list" => {
            let l = reg.sessions.lock().unwrap();
            let labels = reg.labels.lock().unwrap();
            let metas = reg.metas.lock().unwrap();
            let mut v: Vec<Value> = l
                .iter()
                .map(|(id, s)| {
                    json!({"id": id, "running": s.try_wait().is_none(), "idle_ms": s.idle_ms(), "label": labels.get(id), "meta": metas.get(id)})
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
                    let mut wts = axelstra_core::git::worktree_list(repo).unwrap_or_default();
                    // worktrees made by other tools (e.g. Orca) stay out of the list; only the main one and Axelstra's own
                    wts.retain(|w| w.is_main || w.path.replace('\\', "/").contains("/.axelstra-wt/"));
                    let ident = axelstra_core::git::identity(repo);
                    json!({"id": pr.id, "name": ident.display_name, "path": pr.path, "worktrees": wts,
                           "setup": pr.setup, "default_branch": axelstra_core::git::default_branch(repo), "slug": ident.slug, "icon_url": ident.icon_url, "remote": ident.remote})
                })
                .collect();
            Ok(Value::Array(v))
        }
        "git.changes" => {
            let wt = crate::projects::known_worktree(&reg.store, p["worktree"].as_str().unwrap_or_default())?;
            let base = p["base"].as_str();
            Ok(serde_json::to_value(tokio::task::block_in_place(|| crate::changes::changes(&wt, base))?)?)
        }
        "git.log" | "git.show" => {
            let wt = crate::projects::known_worktree(&reg.store, p["worktree"].as_str().unwrap_or_default())?;
            tokio::task::block_in_place(|| -> Result<Value> {
                Ok(if req.method == "git.log" {
                    json!({"commits": crate::changes::log(&wt, p["base"].as_str())?})
                } else {
                    json!({"diff": crate::changes::show(&wt, p["sha"].as_str().unwrap_or_default())?})
                })
            })
        }
        "git.diff" => {
            let wt = crate::projects::known_worktree(&reg.store, p["worktree"].as_str().unwrap_or_default())?;
            let path = p["path"].as_str().ok_or_else(|| anyhow::anyhow!("missing path"))?;
            let base = p["base"].as_str();
            let text = tokio::task::block_in_place(|| crate::changes::diff(&wt, path, base))?;
            Ok(json!({"diff": text}))
        }
        "project.logo" => {
            let pr = crate::projects::find(&reg.store, p["id"].as_str().unwrap_or_default())?;
            let logo = tokio::task::block_in_place(|| crate::projects::logo_data_url(std::path::Path::new(&pr.path)));
            Ok(json!({"data_url": logo}))
        }
        "orch.start" => {
            let pr = crate::projects::find(&reg.store, p["project"].as_str().unwrap_or_default())?;
            if reg.runs.lock().unwrap().get(&pr.id).is_some_and(|r| r["state"] == "running") {
                anyhow::bail!("an orchestration is already running for this project");
            }
            let repo = axelstra_core::git::canon(std::path::Path::new(&pr.path))?;
            let name = format!(
                "ui-{}",
                std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs()
            );
            let max = p["max"].as_u64().unwrap_or(3).clamp(1, 8) as usize;
            // overriding the agent command is for tests and custom agents; the same
            // client can already spawn any process, so this grants nothing new
            let agent: Option<Vec<String>> = p["agent"]
                .as_array()
                .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect());
            // connects back to this same daemon: must not hold an async worker
            let runner = tokio::task::block_in_place(|| crate::runner::DaemonRunner::connect(&reg.pipe, &reg.token))?;
            let started = json!({"state": "running", "name": name, "integ_branch": format!("orch/{name}")});
            reg.runs.lock().unwrap().insert(pr.id.clone(), started.clone());
            let (reg2, pid, n2) = (reg.clone(), pr.id.clone(), name.clone());
            std::thread::spawn(move || {
                let res = axelstra_core::orch::run(&axelstra_core::orch::Options {
                    wt_root: repo.parent().unwrap_or(&repo).join(".axelstra-wt"),
                    repo,
                    name: n2.clone(),
                    max_concurrency: max,
                    agent_override: agent,
                    poll_ms: 500,
                    runner: Some(Arc::new(runner)),
                });
                let v = match res {
                    Ok(r) => json!({"state": "done", "name": n2, "integ_branch": r.integ_branch,
                        "integ_wt": r.integ_wt, "merged": r.merged, "failed": r.failed,
                        "needs_review": r.needs_review, "blocked": r.blocked}),
                    Err(e) => json!({"state": "error", "name": n2, "error": format!("{e:#}")}),
                };
                reg2.runs.lock().unwrap().insert(pid, v);
            });
            Ok(started)
        }
        "orch.status" => {
            let id = p["project"].as_str().unwrap_or_default();
            Ok(reg.runs.lock().unwrap().get(id).cloned().unwrap_or(json!({"state": "idle"})))
        }
        "kanban.list" | "kanban.save" | "kanban.move" | "kanban.delete" => {
            let pr = crate::projects::find(&reg.store, p["project"].as_str().unwrap_or_default())?;
            let repo = std::path::PathBuf::from(&pr.path);
            let s = |k: &str| p[k].as_str().unwrap_or_default().to_string();
            tokio::task::block_in_place(|| -> Result<Value> {
                use crate::kanban;
                Ok(match req.method.as_str() {
                    "kanban.list" => serde_json::to_value(kanban::list(&repo)?)?,
                    "kanban.save" => serde_json::to_value(kanban::save(&repo, p["id"].as_str(), &s("title"), &s("body"), p["status"].as_str(), &p)?)?,
                    "kanban.move" => {
                        kanban::set_status(&repo, &s("id"), &s("status"))?;
                        json!({})
                    }
                    _ => {
                        kanban::delete(&repo, &s("id"))?;
                        json!({})
                    }
                })
            })
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
        // one watch per connection (key u32::MAX in `subs`); a new subscribe replaces the old one
        "watch.subscribe" => {
            let wt = crate::projects::known_worktree(&reg.store, p["worktree"].as_str().unwrap_or_default())?;
            if let Some(old) = subs.remove(&u32::MAX) {
                old.store(true, Ordering::SeqCst);
            }
            let stop = Arc::new(AtomicBool::new(false));
            let (tx2, path) = (tx.clone(), wt.to_string_lossy().to_string());
            tokio::task::block_in_place(|| {
                crate::watcher::start(wt, stop.clone(), move || {
                    let ev = json!({"event": "fs.changed", "worktree": path});
                    let _ = tx2.send(encode(EVENT, ev.to_string().as_bytes()));
                })
            })?;
            subs.insert(u32::MAX, stop);
            Ok(json!({}))
        }
        "watch.unsubscribe" => {
            if let Some(old) = subs.remove(&u32::MAX) {
                old.store(true, Ordering::SeqCst);
            }
            Ok(json!({}))
        }
        "project.setup" => {
            let id = p["id"].as_str().ok_or_else(|| anyhow::anyhow!("missing id"))?;
            crate::projects::set_setup(&reg.store, id, p["setup"].as_str().map(String::from))?;
            Ok(json!({}))
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
        "ports.list" => {
            let wt = crate::projects::known_worktree(&reg.store, p["worktree"].as_str().unwrap_or_default())?;
            let norm = |s: &str| s.replace('\\', "/").to_ascii_lowercase();
            let key = norm(&wt.to_string_lossy());
            let ids: Vec<u32> = {
                let metas = reg.metas.lock().unwrap();
                metas
                    .iter()
                    .filter(|(_, m)| m["worktree"].as_str().map(|w| norm(w)) == Some(key.clone()))
                    .map(|(id, _)| *id)
                    .collect()
            };
            let pids: Vec<u32> =
                ids.iter().filter_map(|id| reg.get(*id)).filter(|s| s.try_wait().is_none()).filter_map(|s| s.pid()).collect();
            Ok(json!({"ports": tokio::task::block_in_place(|| crate::ports::listening(&pids))?}))
        }
        "automation.list" => {
            let _g = reg.auto_lock.lock().unwrap();
            Ok(json!({"automations": crate::automations::load(&crate::automations::store_path(&reg.store))}))
        }
        "automation.save" => {
            let _g = reg.auto_lock.lock().unwrap();
            let path = crate::automations::store_path(&reg.store);
            let mut a: crate::automations::Automation = serde_json::from_value(p.clone())
                .map_err(|e| anyhow::anyhow!("invalid automation: {e}"))?;
            crate::automations::validate(&a)?;
            crate::projects::find(&reg.store, &a.project)?;
            let mut list = crate::automations::load(&path);
            match list.iter_mut().find(|x| !a.id.is_empty() && x.id == a.id) {
                Some(old) => {
                    // editing keeps the run history; a changed schedule starts counting from now
                    if serde_json::to_value(&old.schedule)? != serde_json::to_value(&a.schedule)? {
                        a.last_run = crate::automations::now_ts();
                    } else {
                        a.last_run = old.last_run;
                    }
                    *old = a.clone();
                }
                None => {
                    a.id = format!("auto-{}", crate::automations::now_ts());
                    while list.iter().any(|x| x.id == a.id) {
                        a.id.push('x');
                    }
                    a.last_run = crate::automations::now_ts();
                    list.push(a.clone());
                }
            }
            crate::automations::save_all(&path, &list)?;
            Ok(serde_json::to_value(a)?)
        }
        "automation.delete" => {
            let _g = reg.auto_lock.lock().unwrap();
            let path = crate::automations::store_path(&reg.store);
            let mut list = crate::automations::load(&path);
            list.retain(|x| x.id != p["id"].as_str().unwrap_or_default());
            crate::automations::save_all(&path, &list)?;
            Ok(json!({}))
        }
        "automation.run" => {
            let _g = reg.auto_lock.lock().unwrap();
            let path = crate::automations::store_path(&reg.store);
            tokio::task::block_in_place(|| run_automation(reg, &path, p["id"].as_str().unwrap_or_default()))
        }
        "gh.list" => {
            let pr = crate::projects::find(&reg.store, p["project"].as_str().unwrap_or_default())?;
            let kind = p["kind"].as_str().unwrap_or("pr").to_string();
            let items = tokio::task::block_in_place(|| crate::gh::list(std::path::Path::new(&pr.path), &kind))?;
            Ok(json!({"items": items}))
        }
        "worktree.from_pr" => {
            let pr = crate::projects::find(&reg.store, p["project"].as_str().unwrap_or_default())?;
            let n = p["number"].as_u64().ok_or_else(|| anyhow::anyhow!("missing number"))?;
            Ok(serde_json::to_value(tokio::task::block_in_place(|| crate::projects::worktree_from_pr(&pr, n))?)?)
        }
        "worktree.remove" => {
            let pr = crate::projects::find(&reg.store, p["project"].as_str().unwrap_or_default())?;
            let path = p["path"].as_str().ok_or_else(|| anyhow::anyhow!("missing path"))?;
            tokio::task::block_in_place(|| crate::projects::worktree_remove(&pr, path))?;
            Ok(json!({}))
        }
        "ping" => Ok(json!({})),
        // used by the app updater: the installer cannot replace a running axelstra.exe.
        // Sessions die with the daemon (Job Objects), so the UI warns before calling this.
        "daemon.shutdown" => {
            std::thread::spawn(|| {
                std::thread::sleep(std::time::Duration::from_millis(300));
                std::process::exit(0);
            });
            Ok(json!({}))
        }
        m => anyhow::bail!("unknown method {m}"),
    }
}
