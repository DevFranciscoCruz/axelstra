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
    /// spawn requests of live sessions, kept on disk so a daemon restart can bring them back
    specs: Mutex<HashMap<u32, Value>>,
    /// sessions of the previous daemon run, waiting for the UI to restore (or discard) them
    pending: Mutex<Vec<Value>>,
    saved: std::path::PathBuf,
    /// messages between sessions, worker bookkeeping and the worker limit
    bus: crate::msgbus::Bus,
    /// open connections (the app, CLI calls); a daemon nobody talks to ends itself, see `spawn_idle_exit`
    clients: std::sync::atomic::AtomicUsize,
    /// agents may look at the screen and drive mouse and keyboard (`axelstra computer`); off at every start
    computer: AtomicBool,
}

impl Registry {
    /// Rewrites sessions.json: still-pending sessions first, then the live ones.
    fn persist(&self) {
        let mut v = self.pending.lock().unwrap().clone();
        let specs = self.specs.lock().unwrap();
        let mut ids: Vec<_> = specs.keys().copied().collect();
        ids.sort();
        v.extend(ids.iter().map(|i| specs[i].clone()));
        if let Ok(text) = serde_json::to_string(&v) {
            let _ = std::fs::write(&self.saved, text);
        }
    }

    fn get(&self, id: u32) -> Option<Arc<Session>> {
        self.sessions.lock().unwrap().get(&id).cloned()
    }

    /// Starts a session and records its label and UI metadata.
    fn spawn(&self, s: &Spawn, label: Option<&str>, meta: Option<Value>) -> Result<u32> {
        // the session learns its own id, so a CLI run inside it can name it as the parent of a worker
        let id = self.next.fetch_add(1, Ordering::SeqCst) + 1;
        let mut s = s.clone();
        s.env.push(("AXELSTRA_SESSION".into(), id.to_string()));
        // `axelstra` works in every session without any setup: its folder goes first on the session's PATH
        if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(|p| p.to_path_buf())) {
            let mut paths = vec![dir];
            if let Some(cur) = std::env::var_os("PATH") {
                paths.extend(std::env::split_paths(&cur));
            }
            if let Ok(joined) = std::env::join_paths(paths) {
                s.env.retain(|(k, _)| !k.eq_ignore_ascii_case("PATH"));
                s.env.push(("PATH".into(), joined.to_string_lossy().into_owned()));
            }
        }
        let sess = Arc::new(Session::spawn(&s)?);
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

/// Ephemeral worker: when it finishes (process exit, or quiet for `idle_secs` in "idle" mode) its last
/// output goes to the parent session as if typed, and the worker is closed so it stops using memory.
/// Typical use: a test run or a helper agent that reports back to the main agent and disappears.
fn spawn_reaper(reg: Arc<Registry>, id: u32, parent: Option<u32>, idle_mode: bool, idle_secs: u64, lines: usize) {
    std::thread::spawn(move || {
        let mut worked = false;
        let mut done_by_idle = false;
        loop {
            std::thread::sleep(Duration::from_millis(500));
            let Some(s) = reg.get(id) else { return };
            let exited = s.try_wait();
            let idle = s.idle_ms();
            if idle < 3000 {
                worked = true;
            }
            if exited.is_some() || reg.bus.finished(id).is_some() {
                break;
            }
            if idle_mode && worked && idle >= idle_secs * 1000 && !axelstra_core::waiting::needs_input(&s.tail(4096)) {
                done_by_idle = true;
                break;
            }
        }
        let Some(s) = reg.get(id) else { return };
        // give the output pipe a moment to drain after the process ended
        std::thread::sleep(Duration::from_millis(300));
        let code = s.try_wait();
        let done_msg = reg.bus.finished(id);
        let text = axelstra_core::waiting::strip_ansi(&s.tail(32 * 1024));
        let all: Vec<&str> = text.lines().map(str::trim_end).filter(|l| !l.trim().is_empty()).collect();
        let tail = all[all.len().saturating_sub(lines)..].join("\n");
        let label = reg.labels.lock().unwrap().get(&id).cloned().unwrap_or_else(|| format!("sessão {id}"));
        let status = match (done_by_idle, code) {
            _ if done_msg.is_some() => "terminou (chamou `axelstra done`)".to_string(),
            (true, _) => "terminou (parou de produzir output)".to_string(),
            (_, Some(0)) => "terminou com sucesso (exit 0)".to_string(),
            (_, Some(c)) => format!("terminou com erro (exit {c})"),
            _ => "terminou".to_string(),
        };
        if let Some(parent) = parent.and_then(|p| reg.get(p).map(|s| (p, s))) {
            let agent = reg.metas.lock().unwrap().get(&parent.0).and_then(|m| m["kind"].as_str().map(|k| k == "agent")).unwrap_or(false);
            let body = match &done_msg {
                Some(d) => format!("Relatório:\n{}", d["text"].as_str().unwrap_or_default()),
                None => format!("Últimas linhas:\n{tail}"),
            };
            let msg = format!("[axelstra] worker \"{label}\" {status}. {body}");
            // an agent TUI takes a multi-line message as one paste; a plain shell gets the single line
            let data = if agent { format!("\x1b[200~{msg}\x1b[201~\r") } else { format!("echo [axelstra] worker {label}: {status}\r") };
            let _ = parent.1.write(data.as_bytes());
        }
        s.kill();
        reg.sessions.lock().unwrap().remove(&id);
        reg.labels.lock().unwrap().remove(&id);
        reg.metas.lock().unwrap().remove(&id);
        reg.specs.lock().unwrap().remove(&id);
        reg.bus.forget(id);
        reg.persist();
    });
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
    let saved = store.with_file_name("sessions.json");
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
        specs: Default::default(),
        pending: Mutex::new(
            std::fs::read_to_string(&saved).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default(),
        ),
        saved,
        bus: crate::msgbus::Bus::new(),
        clients: Default::default(),
        computer: AtomicBool::new(false),
    });
    // Claude Code hook: records the agent's current conversation id (it changes on /clear), keyed by AXELSTRA_KEY
    let dir = reg.saved.parent().map(std::path::Path::to_path_buf).unwrap_or_default();
    let hook = json!({"hooks": {"SessionStart": [{"hooks": [{"type": "command",
        "command": "[ -n \"$AXELSTRA_KEY\" ] || exit 0; mkdir -p \"$AXELSTRA_DIR/sid\" && cat > \"$AXELSTRA_DIR/sid/$AXELSTRA_KEY.json\""}]}]}});
    let _ = std::fs::create_dir_all(&dir);
    // the status line command hands Claude's own numbers (context window, 5 h and weekly limits) to the app; it then
    // runs the user's own status line, if they have one, so nothing they see changes
    let mut hook = hook;
    if let Ok(exe) = std::env::current_exe() {
        hook["statusLine"] = json!({"type": "command", "command": format!("\"{}\" statusline-tee", exe.display())});
    }
    let _ = std::fs::write(dir.join("hooks.json"), hook.to_string());
    spawn_scheduler(reg.clone());
    spawn_idle_exit(reg.clone());
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

/// Keeps a stray daemon from lingering: with no connection, no running session and no enabled automation for
/// ten minutes (`AXELSTRA_IDLE_EXIT_SECS`, 0 turns it off) it exits. The app normally stops it on quit.
fn spawn_idle_exit(reg: Arc<Registry>) {
    let limit: u64 = std::env::var("AXELSTRA_IDLE_EXIT_SECS").ok().and_then(|v| v.parse().ok()).unwrap_or(600);
    if limit == 0 {
        return;
    }
    std::thread::spawn(move || {
        let mut idle = 0u64;
        loop {
            std::thread::sleep(Duration::from_secs(10));
            let busy = reg.clients.load(Ordering::SeqCst) > 0
                || reg.sessions.lock().unwrap().values().any(|s| s.try_wait().is_none())
                || crate::automations::load(&crate::automations::store_path(&reg.store)).iter().any(|a| a.enabled);
            idle = if busy { 0 } else { idle + 10 };
            if idle >= limit {
                std::process::exit(0);
            }
        }
    });
}

struct ClientGuard(Arc<Registry>);

impl Drop for ClientGuard {
    fn drop(&mut self) {
        self.0.clients.fetch_sub(1, Ordering::SeqCst);
    }
}

async fn handle(pipe: impl AsyncRead + AsyncWrite + Send + 'static, reg: Arc<Registry>, token: String) {
    reg.clients.fetch_add(1, Ordering::SeqCst);
    let _guard = ClientGuard(reg.clone());
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
            Ok(json!({"proto": PROTO}))
        }
        "session.spawn" => {
            let mut s = Spawn {
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
            let mut meta = if p["meta"].is_null() { None } else { Some(p["meta"].clone()) };
            // a session started on behalf of another one (a worker): remembered as its child
            let parent = p["parent"].as_u64().or_else(|| p["reap"]["parent"].as_u64()).map(|x| x as u32);
            let is_worker = p["reap"].is_object() || p["worker"].as_bool() == Some(true);
            if is_worker {
                let alive = |i: u32| reg.get(i).map(|s| s.try_wait().is_none()).unwrap_or(false);
                let (live, max) = (reg.bus.live_workers(alive), reg.bus.limit());
                if live >= max {
                    anyhow::bail!("worker limit reached ({live} running, max {max}): wait for one to finish or raise it with `axelstra limit N`");
                }
            }
            if let Some(par) = parent {
                let m = meta.get_or_insert_with(|| json!({}));
                m["parent"] = json!(par);
                if is_worker {
                    m["worker"] = json!(true);
                }
                s.env.push(("AXELSTRA_PARENT".into(), par.to_string()));
            }
            let id = reg.spawn(&s, p["label"].as_str(), meta)?;
            if let Some(par) = parent {
                reg.bus.set_parent(id, par);
            }
            if is_worker {
                reg.bus.add_worker(id);
            }
            if let Some(r) = p["reap"].as_object() {
                // ephemeral worker: never restored after a restart, reports to `parent` and closes itself
                reg.bus.mark_reaped(id);
                let parent = r.get("parent").and_then(Value::as_u64).map(|x| x as u32);
                let idle_mode = r.get("mode").and_then(Value::as_str) == Some("idle");
                let idle_secs = r.get("idle_secs").and_then(Value::as_u64).unwrap_or(20).max(3);
                let lines = r.get("lines").and_then(Value::as_u64).unwrap_or(40).clamp(1, 400) as usize;
                spawn_reaper(reg.clone(), id, parent, idle_mode, idle_secs, lines);
                return Ok(json!({"id": id}));
            }
            let mut spec = p.clone();
            spec["id"] = Value::Null;
            reg.specs.lock().unwrap().insert(id, spec);
            reg.persist();
            Ok(json!({"id": id}))
        }
        "session.list" => {
            let l = reg.sessions.lock().unwrap();
            let labels = reg.labels.lock().unwrap();
            let metas = reg.metas.lock().unwrap();
            let mut v: Vec<Value> = l
                .iter()
                .map(|(id, s)| {
                    let running = s.try_wait().is_none();
                    let idle = s.idle_ms();
                    // quiet for a moment and the screen ends in a question: the agent waits for the user
                    let waiting = running && (reg.bus.is_escalated(*id) || (idle >= 1500 && axelstra_core::waiting::needs_input(&s.tail(4096))));
                    json!({"id": id, "running": running, "idle_ms": idle, "waiting": waiting, "escalated": reg.bus.is_escalated(*id), "unread": reg.bus.unread(*id), "finished": reg.bus.finished(*id).is_some(), "label": labels.get(id), "meta": metas.get(id)})
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
            reg.specs.lock().unwrap().remove(&id);
            reg.bus.forget(id);
            reg.persist();
            Ok(json!({}))
        }
        "agent.status" => {
            let key = p["key"].as_str().unwrap_or_default();
            if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
                anyhow::bail!("bad key");
            }
            let file = reg.saved.parent().unwrap_or(std::path::Path::new("")).join("status").join(format!("{key}.json"));
            let status = std::fs::read_to_string(&file).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok());
            Ok(json!({"status": status}))
        }
        "codex.limits" => Ok(json!({"limits": crate::agentlog::codex_limits()})),
        "computer.status" => Ok(json!({"enabled": reg.computer.load(Ordering::SeqCst)})),
        "computer.enable" => {
            reg.computer.store(p["on"].as_bool() == Some(true), Ordering::SeqCst);
            Ok(json!({"enabled": reg.computer.load(Ordering::SeqCst)}))
        }
        "msg.send" => {
            let to = p["to"].as_u64().ok_or_else(|| anyhow::anyhow!("missing to"))? as u32;
            let from = p["from"].as_u64().map(|x| x as u32);
            let kind = p["kind"].as_str().unwrap_or("message");
            if !["message", "ask", "reply", "done", "escalate"].contains(&kind) {
                anyhow::bail!("unknown message kind {kind}");
            }
            let target = reg.get(to).ok_or_else(|| anyhow::anyhow!("no such session {to}"))?;
            let text = p["text"].as_str().unwrap_or_default();
            let m = reg.bus.send(from, to, kind, text, p["reply_to"].as_u64(), p["thread"].as_str().map(String::from));
            // agents get a one-line notice; a worker with a reaper reports through it instead
            let quiet = p["quiet"].as_bool() == Some(true) || (kind == "done" && from.map(|f| reg.bus.is_reaped(f)).unwrap_or(false));
            let agent = reg.metas.lock().unwrap().get(&to).map(|m| m["kind"] == "agent").unwrap_or(false);
            if agent && !quiet {
                let label = from
                    .map(|f| reg.labels.lock().unwrap().get(&f).cloned().unwrap_or_else(|| format!("sessão {f}")))
                    .unwrap_or_else(|| "o utilizador".into());
                let _ = target.write(&crate::msgbus::paste(&crate::msgbus::notice(&m, &label)));
            }
            Ok(json!({"id": m.id}))
        }
        "msg.inbox" => {
            let to = p["session"].as_u64().ok_or_else(|| anyhow::anyhow!("missing session"))? as u32;
            let list = reg.bus.inbox(to, p["all"].as_bool() == Some(true), p["mark"].as_bool() != Some(false));
            Ok(json!(list))
        }
        "msg.thread" => {
            let name = p["name"].as_str().ok_or_else(|| anyhow::anyhow!("missing name"))?;
            Ok(json!(reg.bus.thread(name)))
        }
        "msg.reply_of" => {
            let id = p["id"].as_u64().ok_or_else(|| anyhow::anyhow!("missing id"))?;
            Ok(json!({"reply": reg.bus.reply_to(id), "original": reg.bus.get(id)}))
        }
        "orch.limits" => {
            if let Some(n) = p["max_workers"].as_u64() {
                reg.bus.set_limit(n as usize);
            }
            let alive = |i: u32| reg.get(i).map(|s| s.try_wait().is_none()).unwrap_or(false);
            Ok(json!({"max_workers": reg.bus.limit(), "workers": reg.bus.live_workers(alive)}))
        }
        "app.dir" => Ok(json!({"dir": reg.saved.parent().unwrap_or(std::path::Path::new("")).to_string_lossy().replace('\\', "/")})),
        "session.pending" => {
            let sid_dir = reg.saved.with_file_name("sid");
            let mut v = reg.pending.lock().unwrap().clone();
            for rec in v.iter_mut() {
                // the agent may have started a new conversation (/clear) since: resume the latest one
                let Some(key) = rec["meta"]["key"].as_str().map(String::from) else { continue };
                let latest = std::fs::read_to_string(sid_dir.join(format!("{key}.json")))
                    .ok()
                    .and_then(|t| serde_json::from_str::<Value>(&t).ok())
                    .and_then(|j| j["session_id"].as_str().map(String::from));
                if let Some(id) = latest {
                    // a conversation with no replies (cleared, never used) is not worth resuming: start clean
                    let cwd = rec["cwd"].as_str().or(rec["meta"]["worktree"].as_str()).unwrap_or("");
                    let empty = crate::agentlog::claude_usage(cwd, Some(&id)).map(|u| u.messages == 0).unwrap_or(true);
                    if empty {
                        rec["meta"]["blank"] = Value::Bool(true);
                    }
                    rec["meta"]["resume"] = Value::String(id);
                }
            }
            Ok(Value::Array(v))
        }
        "session.pending_clear" => {
            reg.pending.lock().unwrap().clear();
            reg.persist();
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
        "worktree.status" => {
            // only paths the daemon knows; the list comes from the UI and git runs in each
            let list: Vec<String> = p["paths"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default();
            let norm = |s: &str| s.replace('\\', "/").to_ascii_lowercase();
            let out = tokio::task::block_in_place(|| {
                let mut allowed = std::collections::HashSet::new();
                for pr in crate::projects::load(&reg.store) {
                    for w in axelstra_core::git::worktree_list(std::path::Path::new(&pr.path)).unwrap_or_default() {
                        allowed.insert(norm(&w.path));
                    }
                }
                list.iter().take(64).filter(|w| allowed.contains(&norm(w))).map(|w| crate::wtstatus::status(w)).collect::<Vec<_>>()
            });
            Ok(json!({"status": out}))
        }
        "agent.search" => {
            let q = p["query"].as_str().unwrap_or_default().to_string();
            let max = p["max"].as_u64().unwrap_or(30).clamp(1, 200) as usize;
            let hits = tokio::task::block_in_place(|| crate::agentlog::search(&q, max));
            Ok(json!({"hits": hits}))
        }
        "agent.usage" => {
            let cwd = p["cwd"].as_str().unwrap_or_default().to_string();
            let u = tokio::task::block_in_place(|| crate::agentlog::claude_usage(&cwd, p["session"].as_str()));
            Ok(json!({"usage": u}))
        }
        "project.list" => {
            let ps = crate::projects::load(&reg.store);
            let v: Vec<Value> = ps
                .iter()
                .map(|pr| {
                    let repo = std::path::Path::new(&pr.path);
                    let mut wts = axelstra_core::git::worktree_list(repo).unwrap_or_default();
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
        "fs.list" | "fs.read" | "fs.stat" | "fs.write" | "fs.search" | "fs.grep" | "fs.create" | "fs.rename" | "fs.delete" => {
            let wt = crate::projects::known_worktree(&reg.store, p["worktree"].as_str().unwrap_or_default())?;
            let path = p["path"].as_str().unwrap_or_default();
            tokio::task::block_in_place(|| -> Result<Value> {
                use crate::fsops;
                Ok(match req.method.as_str() {
                    "fs.list" => json!({"entries": fsops::list(&wt, path)?}),
                    "fs.grep" => json!({"matches": fsops::grep(&wt, p["query"].as_str().unwrap_or_default())?}),
                    "fs.create" => {
                        fsops::create(&wt, path, p["dir"].as_bool().unwrap_or(false))?;
                        json!({})
                    }
                    "fs.rename" => {
                        fsops::rename(&wt, path, p["to"].as_str().unwrap_or_default())?;
                        json!({})
                    }
                    "fs.delete" => {
                        fsops::delete(&wt, path)?;
                        json!({})
                    }
                    "fs.search" => json!({"files": fsops::search(&wt, p["query"].as_str().unwrap_or_default())?}),
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
        "open.editor" => {
            let wt = crate::projects::known_worktree(&reg.store, p["worktree"].as_str().unwrap_or_default())?;
            // only editors we know; never a program name from the client
            let (prog, args): (&str, Vec<String>) = match p["app"].as_str().unwrap_or_default() {
                "code" if cfg!(windows) => ("cmd", vec!["/d".into(), "/c".into(), "code".into(), wt.to_string_lossy().into_owned()]),
                "code" => ("code", vec![wt.to_string_lossy().into_owned()]),
                other => anyhow::bail!("unknown editor {other:?}"),
            };
            axelstra_core::git::quiet(&mut std::process::Command::new(prog))
                .args(args)
                .spawn()
                .map_err(|e| anyhow::anyhow!("could not start {prog}: {e}"))?;
            Ok(json!({}))
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
            // end every terminal and agent now (their specs stay in sessions.json, so they restore on the next start)
            for s in reg.sessions.lock().unwrap().values() {
                s.kill();
            }
            std::thread::spawn(|| {
                std::thread::sleep(std::time::Duration::from_millis(300));
                std::process::exit(0);
            });
            Ok(json!({}))
        }
        m => anyhow::bail!("unknown method {m}"),
    }
}

