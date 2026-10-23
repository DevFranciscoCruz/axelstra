use crate::proto::*;
use crate::store::{read_json, write_atomic, LockRecover};
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

/// Identifies this daemon run: a client that sees it change knows the daemon restarted (its sessions are gone).
static INSTANCE: std::sync::LazyLock<u64> = std::sync::LazyLock::new(|| {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
});

/// Frames queued for one connection's writer task. Bounded: a client that stops reading slows its own senders
/// down (they block) instead of growing the daemon's memory.
type Tx = mpsc::Sender<Vec<u8>>;
const OUT_QUEUE: usize = 1024;

/// Lock order. Hold at most these in this order, never a later one while taking an earlier one:
///   auto_lock, sessions, labels, metas, specs, pending, then the leaf locks (runs, flows, plist).
/// Leaf locks are never held while taking another lock. A path that needs two takes them in this order
/// (`session.list`, `daemon.shutdown` and `persist` do); the `lock_order` test runs those paths against each other.
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
    /// running `axelstra flow` plans: flow id -> [{id, worktree, state}] (what the sidebar shows as queued/running/done)
    flows: Mutex<HashMap<String, Value>>,
    /// project.list answer reused for a few seconds (it starts several git processes per project)
    plist: Mutex<Option<(std::time::Instant, Value)>>,
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
        // both locks stay held through the write, so two persists cannot land out of order
        let specs = self.specs.lock_recover();
        let pending = self.pending.lock_recover();
        let mut v = pending.clone();
        let mut ids: Vec<_> = specs.keys().copied().collect();
        ids.sort();
        v.extend(ids.iter().map(|i| specs[i].clone()));
        if let Ok(text) = serde_json::to_string(&v) {
            let _ = write_atomic(&self.saved, text.as_bytes());
        }
    }

    fn new(store: std::path::PathBuf, pipe: String, token: String) -> Registry {
        let saved = store.with_file_name("sessions.json");
        // a corrupt sessions.json is moved to sessions.json.bad, so the first persist cannot wipe it
        let pending = read_json::<Vec<Value>>(&saved).ok().flatten().unwrap_or_default();
        Registry {
            next: AtomicU32::new(0),
            sessions: Default::default(),
            labels: Default::default(),
            metas: Default::default(),
            store,
            pipe,
            token,
            runs: Default::default(),
            auto_lock: Default::default(),
            flows: Default::default(),
            plist: Default::default(),
            specs: Default::default(),
            pending: Mutex::new(pending),
            saved,
            bus: crate::msgbus::Bus::new(),
            clients: Default::default(),
            computer: AtomicBool::new(false),
        }
    }

    /// True while a live session works in this worktree (any terminal or agent with it as worktree or cwd).
    fn worktree_busy(&self, path: &str) -> bool {
        let norm = |s: &str| s.replace('\\', "/").trim_end_matches('/').to_ascii_lowercase();
        let want = norm(path);
        // a worktree a running flow plan still has to use (queued or running) is not finished, whatever its branch says
        let in_flow = self.flows.lock_recover().values().any(|t| {
            t.as_array()
                .map(|a| {
                    a.iter().any(|x| {
                        x["worktree"].as_str().map(|w| norm(w) == want).unwrap_or(false)
                            && matches!(x["state"].as_str(), Some("queued") | Some("running"))
                    })
                })
                .unwrap_or(false)
        });
        if in_flow {
            return true;
        }
        // one lock at a time (the rest of the daemon takes sessions before metas)
        let alive: Vec<u32> = self.sessions.lock_recover().iter().filter(|(_, s)| s.try_wait().is_none()).map(|(id, _)| *id).collect();
        let metas = self.metas.lock_recover();
        let in_meta = |id: &u32| metas.get(id).and_then(|m| m["worktree"].as_str()).map(|w| norm(w) == want).unwrap_or(false);
        let hit = alive.iter().any(in_meta);
        drop(metas);
        let specs = self.specs.lock_recover();
        hit || alive.iter().any(|id| specs.get(id).and_then(|sp| sp["cwd"].as_str()).map(|c| within(&norm(c), &want)).unwrap_or(false))
    }

    /// True while a live session in this worktree is not idle (output in the last 5 s). A prompt waiting for
    /// input does not count: renaming the worktree is fine then.
    fn worktree_working(&self, path: &str) -> bool {
        let norm = |s: &str| s.replace('\\', "/").trim_end_matches('/').to_ascii_lowercase();
        let want = norm(path);
        let working: Vec<u32> =
            self.sessions.lock_recover().iter().filter(|(_, s)| s.try_wait().is_none() && s.idle_ms() < 5000).map(|(id, _)| *id).collect();
        let metas = self.metas.lock_recover();
        working.iter().any(|id| metas.get(id).and_then(|m| m["worktree"].as_str()).map(|w| norm(w) == want).unwrap_or(false))
    }

    fn get(&self, id: u32) -> Option<Arc<Session>> {
        self.sessions.lock_recover().get(&id).cloned()
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
        self.sessions.lock_recover().insert(id, sess);
        if let Some(l) = label {
            self.labels.lock_recover().insert(id, l.to_string());
        }
        if let Some(m) = meta {
            self.metas.lock_recover().insert(id, m);
        }
        Ok(id)
    }
}

/// True when `path` is `root` or lies below it, comparing whole path components (`/a/wt-2` is not inside `/a/wt`).
/// Both are already normalized (forward slashes, lowercase).
fn within(path: &str, root: &str) -> bool {
    std::path::Path::new(path).starts_with(root)
}

const SHELLS: [&str; 6] = ["auto", "pwsh", "powershell", "bash", "wsl", "cmd"];
const NOTIFY_MODES: [&str; 3] = ["all", "waiting", "off"];

/// `shell`, `notify_sound` and `notify_mode` as saved, each falling back to its default (auto, true, all) when missing or invalid.
/// The app itself reads the same keys from settings.json.
fn ui_settings(saved: &Value) -> (&str, bool, &str) {
    let shell = saved["shell"].as_str().filter(|s| SHELLS.contains(s)).unwrap_or("auto");
    let mode = saved["notify_mode"].as_str().filter(|m| NOTIFY_MODES.contains(m)).unwrap_or("all");
    (shell, saved["notify_sound"] != json!(false), mode)
}

/// Copies the keys of a `settings.set` patch that are present into `saved`, rejecting invalid values.
fn apply_ui_settings(saved: &mut Value, p: &Value) -> Result<()> {
    if let Some(s) = p["shell"].as_str() {
        if !SHELLS.contains(&s) {
            return Err(anyhow::anyhow!("shell must be auto, pwsh, powershell, bash, wsl or cmd"));
        }
        saved["shell"] = json!(s);
    }
    if let Some(b) = p["notify_sound"].as_bool() {
        saved["notify_sound"] = json!(b);
    }
    if let Some(m) = p["notify_mode"].as_str() {
        if !NOTIFY_MODES.contains(&m) {
            return Err(anyhow::anyhow!("notify_mode must be all, waiting or off"));
        }
        saved["notify_mode"] = json!(m);
    }
    Ok(())
}

/// The app-wide settings file next to the daemon state (missing or broken = no settings).
fn read_settings(saved: &std::path::Path) -> Value {
    read_json(&saved.with_file_name("settings.json")).ok().flatten().unwrap_or_else(|| json!({}))
}

/// Ephemeral worker: when it finishes (process exit, or quiet for `idle_secs` in "idle" mode) its last
/// output goes to the parent session as if typed, and the worker is closed so it stops using memory.
/// Typical use: a test run or a helper agent that reports back to the main agent and disappears.
fn spawn_reaper(reg: Arc<Registry>, id: u32, parent: Option<u32>, idle_mode: bool, idle_secs: u64, lines: usize) {
    std::thread::spawn(move || {
        let mut worked = false;
        let mut done_by_idle = false;
        let mut nudged = false;
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
            if idle_mode && worked && idle >= idle_secs * 1000 {
                let screen = s.screen_lines(12);
                if axelstra_core::waiting::needs_input(&s.tail(4096)) || axelstra_core::waiting::asks_question(&screen) {
                    // it is waiting for an answer, not finished: keep it and tell the parent once (until it works again)
                    if let Some((pid, ps)) = parent.and_then(|p| reg.get(p).map(|s| (p, s))) {
                        let label = reg.labels.lock_recover().get(&id).cloned().unwrap_or_else(|| format!("sessão {id}"));
                        let shown = axelstra_core::screen::last_reply(&s.screen_lines(200)).unwrap_or_else(|| screen.join("\n"));
                        let text = format!("parou à espera de uma resposta. Última mensagem:\n{shown}");
                        let m = reg.bus.send(Some(id), pid, "escalate", &text, None, None);
                        let agent = reg.metas.lock_recover().get(&pid).map(|m| m["kind"] == "agent").unwrap_or(false);
                        if agent {
                            crate::msgbus::deliver(ps, crate::msgbus::paste(&crate::msgbus::notice(&m, &format!("{label} (sessão {id})"))));
                        }
                    }
                    worked = false;
                } else if !nudged && reg.metas.lock_recover().get(&id).map(|m| m["kind"] == "agent").unwrap_or(false) {
                    // first quiet spell: ask the worker to report through the bus instead of scraping its screen
                    let text = "[axelstra] você parou sem reportar. Se terminou, rode `axelstra done \"<relatório>\"` com o resultado (o que mudou, o que falhou, o que falta). Se está bloqueado ou tem uma dúvida, rode `axelstra escalate \"<motivo>\"`.";
                    crate::msgbus::deliver(s.clone(), crate::msgbus::paste(text));
                    nudged = true;
                    worked = false;
                } else {
                    done_by_idle = true;
                    break;
                }
            }
        }
        let Some(s) = reg.get(id) else { return };
        // give the output pipe a moment to drain after the process ended
        std::thread::sleep(Duration::from_millis(300));
        let code = s.try_wait();
        let done_msg = reg.bus.finished(id);
        // what the terminal shows, not the escape-code soup of a TUI redrawing itself
        // for an agent TUI only its last reply, not the banner, tool-run lines and status bar around it
        let tail = axelstra_core::screen::last_reply(&s.screen_lines(200)).unwrap_or_else(|| s.screen_lines(lines).join("\n"));
        let label = reg.labels.lock_recover().get(&id).cloned().unwrap_or_else(|| format!("sessão {id}"));
        let status = match (done_by_idle, code) {
            _ if done_msg.is_some() => "terminou (chamou `axelstra done`)".to_string(),
            (true, _) => "foi fechado por ficar parado sem chamar `axelstra done` (se o texto abaixo for uma pergunta, ele estava à espera de uma resposta e não concluiu)".to_string(),
            (_, Some(0)) => "terminou com sucesso (exit 0)".to_string(),
            (_, Some(c)) => format!("terminou com erro (exit {c})"),
            _ => "terminou".to_string(),
        };
        if let Some(parent) = parent.and_then(|p| reg.get(p).map(|s| (p, s))) {
            let agent = reg.metas.lock_recover().get(&parent.0).and_then(|m| m["kind"].as_str().map(|k| k == "agent")).unwrap_or(false);
            let body = match &done_msg {
                Some(d) => format!("Relatório:\n{}", d["text"].as_str().unwrap_or_default()),
                None => format!("Últimas linhas:\n{tail}"),
            };
            let msg = format!("[axelstra] worker \"{label}\" {status}. {body}");
            // an agent TUI takes a multi-line message as one paste; a plain shell gets the single line
            let data = if agent {
                crate::msgbus::paste(&msg)
            } else {
                format!("echo [axelstra] worker {}: {}\r", crate::msgbus::echo_safe(&label), crate::msgbus::echo_safe(&status)).into_bytes()
            };
            crate::msgbus::deliver(parent.1.clone(), data);
        }
        s.kill();
        reg.sessions.lock_recover().remove(&id);
        reg.labels.lock_recover().remove(&id);
        reg.metas.lock_recover().remove(&id);
        reg.specs.lock_recover().remove(&id);
        reg.bus.forget(id);
        reg.persist();
    });
}

/// Wakes every 30 s and starts the automations that are due. Missed runs are not replayed.
fn spawn_scheduler(reg: Arc<Registry>) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(30));
        let mut n = 0u32;
        loop {
            tick.tick().await;
            n += 1;
            if n == 1 {
                // folders parked by worktree removals in an earlier run
                let r = reg.clone();
                let _ = tokio::task::spawn_blocking(move || {
                    for pr in crate::projects::load(&r.store) {
                        for w in axelstra_core::git::worktree_list(std::path::Path::new(&pr.path)).unwrap_or_default() {
                            if let Some(parent) = std::path::Path::new(&w.path).parent() {
                                if !w.is_main {
                                    axelstra_core::git::purge_trash(parent);
                                }
                            }
                        }
                    }
                })
                .await;
            }
            if n % 2 == 0 {
                // a crash or a kill skips the shutdown save: keep the agent terminals on disk every minute too
                let r = reg.clone();
                let _ = tokio::task::spawn_blocking(move || save_histories(&r)).await;
            }
            if n % 20 == 0 && !reg.saved.with_file_name("janitor.off").exists() {
                let r = reg.clone();
                let _ = tokio::task::spawn_blocking(move || {
                    for s in crate::janitor::sweep(&r.store, &|path| r.worktree_busy(path), false) {
                        eprintln!("janitor: {s}");
                    }
                })
                .await;
            }
            let r = reg.clone();
            let _ = tokio::task::spawn_blocking(move || {
                let _g = r.auto_lock.lock_recover();
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
    let reg = Arc::new(Registry::new(store, pipe.clone(), token.clone()));
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
    // every instance gets the same explicit DACL (current user and SYSTEM), and remote clients are refused
    let sec = crate::pipesec::PipeSecurity::current_user()?;
    let make = |first: bool| -> std::io::Result<tokio::net::windows::named_pipe::NamedPipeServer> {
        let mut attrs = sec.attributes();
        unsafe {
            ServerOptions::new()
                .first_pipe_instance(first)
                .reject_remote_clients(true)
                .create_with_security_attributes_raw(pipe, (&mut attrs as *mut windows_sys::Win32::Security::SECURITY_ATTRIBUTES).cast())
        }
    };
    let mut server = make(true)?;
    loop {
        // one failed accept (a client that vanished mid-connect) must not end the daemon: log, back off, go on
        if let Err(e) = server.connect().await {
            eprintln!("accept failed: {e}");
            tokio::time::sleep(Duration::from_millis(100)).await;
            server = next_instance(&make).await;
            continue;
        }
        let conn = server;
        server = next_instance(&make).await;
        tokio::spawn(handle(conn, reg.clone(), token.clone()));
    }
}

/// A fresh pipe instance to accept on (same DACL as the first one), retrying with a short backoff while the system
/// refuses to make one.
#[cfg(windows)]
async fn next_instance(
    make: &impl Fn(bool) -> std::io::Result<tokio::net::windows::named_pipe::NamedPipeServer>,
) -> tokio::net::windows::named_pipe::NamedPipeServer {
    loop {
        match make(false) {
            Ok(s) => return s,
            Err(e) => {
                eprintln!("could not create a pipe instance: {e}");
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
        }
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
        match listener.accept().await {
            Ok((conn, _)) => {
                tokio::spawn(handle(conn, reg.clone(), token.clone()));
            }
            Err(e) => {
                eprintln!("accept failed: {e}");
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
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
                || reg.sessions.lock_recover().values().any(|s| s.try_wait().is_none())
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

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

async fn handle(pipe: impl AsyncRead + AsyncWrite + Send + 'static, reg: Arc<Registry>, token: String) {
    reg.clients.fetch_add(1, Ordering::SeqCst);
    let _guard = ClientGuard(reg.clone());
    let (mut rd, mut wr) = tokio::io::split(pipe);
    let (tx, mut rx) = mpsc::channel::<Vec<u8>>(OUT_QUEUE);
    tokio::spawn(async move {
        while let Some(f) = rx.recv().await {
            if wr.write_all(&f).await.is_err() {
                break;
            }
        }
    });

    let mut authed = false;
    // set by a `hello` that says it comes from the CLI (what agents run); such a connection cannot switch screen control on
    let mut cli = false;
    let mut subs: HashMap<u32, Arc<AtomicBool>> = HashMap::new();

    loop {
        // a connection that has not sent a valid `hello` yet must do so quickly, or it holds a task for nothing
        let frame = if authed {
            read_frame(&mut rd).await
        } else {
            match tokio::time::timeout(HANDSHAKE_TIMEOUT, read_frame(&mut rd)).await {
                Ok(f) => f,
                Err(_) => break,
            }
        };
        let Ok((kind, payload)) = frame else { break };
        match kind {
            JSON => {
                let Ok(req) = serde_json::from_slice::<Req>(&payload) else { break };
                if !authed && req.method != "hello" {
                    break;
                }
                let mut after: Vec<Box<dyn FnOnce() + Send>> = Vec::new();
                // dispatch does blocking work (git, process spawns, files): off the async workers, of which there are few
                let t0 = std::time::Instant::now();
                // a bug in one handler must not take every terminal down with the daemon
                let res = tokio::task::block_in_place(|| {
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        dispatch(&req, &reg, &tx, &mut subs, &mut authed, &mut cli, &token, &mut after)
                    }))
                    .unwrap_or_else(|_| Err(anyhow::anyhow!("erro interno do daemon em {} (ver crash.log)", req.method)))
                });
                // calls that wait on purpose are not slow
                if !matches!(req.method.as_str(), "ask" | "msg.ask" | "session.wait" | "hello") {
                    crate::lag::note(&format!("daemon {}", req.method), t0.elapsed(), 30);
                }
                let body = match res {
                    Ok(v) => json!({"id": req.id, "result": v}),
                    Err(e) => json!({"id": req.id, "error": format!("{e:#}")}),
                };
                let _ = tx.send(encode(JSON, body.to_string().as_bytes())).await;
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
                        let t0 = std::time::Instant::now();
                        tokio::task::block_in_place(|| {
                            let _ = s.write(data);
                        });
                        crate::lag::note("daemon pty write", t0.elapsed(), 20);
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

fn history_file(reg: &Registry, key: &str) -> Option<std::path::PathBuf> {
    // the key is a uuid; anything else would be a path
    key.chars().all(|c| c.is_ascii_alphanumeric() || c == '-').then(|| reg.saved.with_file_name("scrollback").join(format!("{key}.txt")))
}

fn save_history(reg: &Registry, key: &str, lines: &[String]) {
    let Some(f) = history_file(reg, key) else { return };
    if lines.is_empty() {
        return;
    }
    let _ = write_atomic(&f, lines.join("\n").as_bytes());
}

/// The saved history as terminal text (dim, with a header), removed from disk once read.
fn take_history(reg: &Registry, key: &str) -> Option<String> {
    let f = history_file(reg, key)?;
    let text = std::fs::read_to_string(&f).ok()?;
    let _ = std::fs::remove_file(&f);
    let body = text.lines().collect::<Vec<_>>().join("\r\n");
    Some(format!("\x1b[2m── histórico anterior ──\r\n{body}\r\n── fim do histórico ──\x1b[0m\r\n"))
}

/// Tells a watching client when the state of any session changed (started, exited, went quiet, escalated, got a
/// message...), so the UI can drop its fast polling. Cheap: it only looks at in-memory counters once a second.
fn ensure_session_notifier(reg: &Arc<Registry>, tx: &Tx, subs: &mut HashMap<u32, Arc<AtomicBool>>) {
    const KEY: u32 = u32::MAX - 1;
    if subs.contains_key(&KEY) {
        return;
    }
    let stop = Arc::new(AtomicBool::new(false));
    subs.insert(KEY, stop.clone());
    let (reg, tx) = (reg.clone(), tx.clone());
    std::thread::spawn(move || {
        let sig = || {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            let sessions = reg.sessions.lock_recover();
            let mut ids: Vec<_> = sessions.keys().copied().collect();
            ids.sort();
            for id in ids {
                let s = &sessions[&id];
                (
                    id,
                    s.try_wait().is_none(),
                    s.idle_ms() >= 1500,
                    reg.bus.is_escalated(id),
                    reg.bus.unread(id),
                    reg.bus.finished(id).is_some(),
                )
                    .hash(&mut h);
            }
            h.finish()
        };
        // the client lists sessions itself when it connects: only changes after that are news
        let mut last = sig();
        while !stop.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(1000));
            let now = sig();
            if now != last {
                last = now;
                if tx.blocking_send(encode(EVENT, json!({"event": "session.changed"}).to_string().as_bytes())).is_err() {
                    return;
                }
            }
        }
    });
}

/// What `session.list` answers. Takes sessions, labels, metas in the registry's lock order.
fn session_list(reg: &Registry) -> Vec<Value> {
    let l = reg.sessions.lock_recover();
    let labels = reg.labels.lock_recover();
    let metas = reg.metas.lock_recover();
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
    v
}

/// Writes the screen of every session that has a history key (agents), for the next start to show dimmed.
/// The locks (sessions, then metas) are held only to pick the sessions; rendering the screens happens after.
fn save_histories(reg: &Registry) {
    let picked: Vec<(String, Arc<Session>)> = {
        let sessions = reg.sessions.lock_recover();
        let metas = reg.metas.lock_recover();
        sessions.iter().filter_map(|(id, s)| metas.get(id).and_then(|m| m["key"].as_str()).map(|k| (k.to_string(), s.clone()))).collect()
    };
    for (key, s) in picked {
        save_history(reg, &key, &s.screen_lines(300));
    }
}

fn dispatch(
    req: &Req,
    reg: &Arc<Registry>,
    tx: &Tx,
    subs: &mut HashMap<u32, Arc<AtomicBool>>,
    authed: &mut bool,
    cli: &mut bool,
    token: &str,
    after: &mut Vec<Box<dyn FnOnce() + Send>>,
) -> Result<Value> {
    let p = &req.params;
    // anything that changes projects or worktrees makes the cached project list stale
    if (req.method.starts_with("worktree.") && req.method != "worktree.status")
        || (req.method.starts_with("project.") && !matches!(req.method.as_str(), "project.list" | "project.logo" | "project.log"))
    {
        *reg.plist.lock_recover() = None;
    }
    let sid = || -> Result<u32> { Ok(p.get("id").and_then(Value::as_u64).ok_or_else(|| anyhow::anyhow!("missing id"))? as u32) };
    let session = || -> Result<Arc<Session>> { reg.get(sid()?).ok_or_else(|| anyhow::anyhow!("no such session")) };
    match req.method.as_str() {
        "hello" => {
            if p.get("token").and_then(Value::as_str) != Some(token) {
                anyhow::bail!("bad token");
            }
            *authed = true;
            *cli = p.get("client").and_then(Value::as_str) == Some("cli");
            Ok(json!({"proto": PROTO, "instance": *INSTANCE}))
        }
        "session.spawn" => {
            let mut s = Spawn {
                program: p["program"].as_str().ok_or_else(|| anyhow::anyhow!("missing program"))?.into(),
                args: p["args"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default(),
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
                    anyhow::bail!(
                        "worker limit reached ({live} running, max {max}): wait for one to finish or raise it with `axelstra limit N`"
                    );
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
            let history_key = p["meta"]["key"].as_str().map(String::from);
            let id = reg.spawn(&s, p["label"].as_str(), meta)?;
            // a restored agent shows what its terminal said before the restart, dimmed, above its own output
            if let (Some(key), Some(sess)) = (history_key, reg.get(id)) {
                if let Some(text) = take_history(&reg, &key) {
                    sess.seed(text.as_bytes());
                }
            }
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
            reg.specs.lock_recover().insert(id, spec);
            reg.persist();
            Ok(json!({"id": id}))
        }
        "session.list" => Ok(Value::Array(session_list(reg))),
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
                        let _ = tx.blocking_send(session_frame(OUTPUT, id, &snap));
                    }
                    let mut quiet_ticks = 0u32;
                    loop {
                        if stop.load(Ordering::SeqCst) {
                            return;
                        }
                        match rx.recv_timeout(Duration::from_millis(200)) {
                            Ok(d) => {
                                quiet_ticks = 0;
                                if tx.blocking_send(session_frame(OUTPUT, id, &d)).is_err() {
                                    return;
                                }
                            }
                            Err(RecvTimeoutError::Timeout) => {
                                // an empty frame every 5 quiet seconds: a client that stopped listening (its tab closed) wakes up,
                                // sees it is detached and hangs up, instead of staying blocked until this session prints again
                                quiet_ticks += 1;
                                if quiet_ticks >= 25 {
                                    quiet_ticks = 0;
                                    if tx.blocking_send(session_frame(OUTPUT, id, &[])).is_err() {
                                        return;
                                    }
                                }
                                // ConPTY may keep the output pipe open after the process is gone
                                if s.try_wait().is_some() {
                                    while let Ok(d) = rx.recv_timeout(Duration::from_millis(150)) {
                                        let _ = tx.blocking_send(session_frame(OUTPUT, id, &d));
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
                    let _ = tx.blocking_send(encode(EVENT, ev.to_string().as_bytes()));
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
        // the tab name the user chose: kept with the session (and in sessions.json, so a restore reopens it under that name)
        "session.rename" => {
            let id = sid()?;
            session()?;
            let wanted: String = p["label"].as_str().unwrap_or_default().trim().chars().take(120).collect();
            let mut labels = reg.labels.lock_recover();
            let mut specs = reg.specs.lock_recover();
            // the name the session was started with, kept the first time it is renamed, so an empty name can bring it back
            let base = specs.get(&id).and_then(|sp| sp["base_label"].as_str().map(str::to_string)).or_else(|| labels.get(&id).cloned());
            let label = if wanted.is_empty() { base.clone().unwrap_or_default() } else { wanted };
            if label.is_empty() {
                labels.remove(&id);
            } else {
                labels.insert(id, label.clone());
            }
            if let Some(spec) = specs.get_mut(&id) {
                if spec.get("base_label").is_none() {
                    if let Some(b) = &base {
                        spec["base_label"] = json!(b);
                    }
                }
                if label.is_empty() {
                    spec.as_object_mut().map(|o| o.remove("label"));
                } else {
                    spec["label"] = json!(label);
                }
            }
            drop(labels);
            drop(specs);
            reg.persist();
            Ok(json!({"label": label}))
        }
        "session.remove" => {
            let id = sid()?;
            if let Some(stop) = subs.remove(&id) {
                stop.store(true, Ordering::SeqCst);
            }
            if let Some(s) = reg.sessions.lock_recover().remove(&id) {
                s.kill();
            }
            reg.labels.lock_recover().remove(&id);
            // a closed tab keeps no history: the periodic save would otherwise leave its file behind
            if let Some(f) = reg.metas.lock_recover().remove(&id).and_then(|m| m["key"].as_str().and_then(|k| history_file(reg, k))) {
                let _ = std::fs::remove_file(f);
            }
            reg.specs.lock_recover().remove(&id);
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
        "worktree.sweep" => {
            let dry = p["dry_run"].as_bool().unwrap_or(false);
            let done = tokio::task::block_in_place(|| crate::janitor::sweep(&reg.store, &|path| reg.worktree_busy(path), dry));
            Ok(json!({"swept": done}))
        }
        "janitor.auto" => {
            // on unless the user turned it off: the file records the opt-out
            let flag = reg.saved.with_file_name("janitor.off");
            if let Some(on) = p["on"].as_bool() {
                if on {
                    let _ = std::fs::remove_file(&flag);
                } else {
                    let _ = std::fs::write(&flag, "1");
                }
            }
            Ok(json!({"on": !flag.exists()}))
        }
        "flow.set" => {
            let id = p["flow"].as_str().unwrap_or_default().to_string();
            if !id.is_empty() {
                reg.flows.lock_recover().insert(id, p["tasks"].clone());
            }
            Ok(json!({}))
        }
        "flow.clear" => {
            reg.flows.lock_recover().remove(p["flow"].as_str().unwrap_or_default());
            Ok(json!({}))
        }
        "flow.list" => {
            let all: Vec<Value> = reg.flows.lock_recover().values().flat_map(|t| t.as_array().cloned().unwrap_or_default()).collect();
            Ok(json!({"tasks": all}))
        }
        "codex.limits" => Ok(json!({"limits": tokio::task::block_in_place(crate::agentlog::codex_limits)})),
        "computer.status" => Ok(json!({"enabled": reg.computer.load(Ordering::SeqCst)})),
        "computer.enable" => {
            // agents run the CLI: only the app (a connection that did not say it is the CLI) may turn it on
            if *cli && p["on"].as_bool() == Some(true) {
                anyhow::bail!("screen control can only be turned on by the user in the app");
            }
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
            let agent = reg.metas.lock_recover().get(&to).map(|m| m["kind"] == "agent").unwrap_or(false);
            if agent && !quiet {
                let label = from
                    .map(|f| reg.labels.lock_recover().get(&f).cloned().unwrap_or_else(|| format!("sessão {f}")))
                    .unwrap_or_else(|| "o utilizador".into());
                crate::msgbus::deliver(target.clone(), crate::msgbus::paste(&crate::msgbus::notice(&m, &label)));
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
        "settings.get" => {
            let saved = read_settings(&reg.saved);
            // `terse` in the request asks for the rules of that level instead of the saved one (a project's own choice)
            let terse = p["terse"].as_str().or(saved["terse"].as_str()).filter(|l| ["off", "short", "tiny"].contains(l)).unwrap_or("off");
            let installed = tokio::task::block_in_place(|| crate::projects::package_managers(None, None).0);
            let pm = saved["pm"].as_str().filter(|m| installed.contains(m)).map(String::from);
            let (shell, notify_sound, notify_mode) = ui_settings(&saved);
            Ok(
                json!({"terse": terse, "rules": crate::skill::terse_rules(terse), "pm": pm, "installed": installed, "auto_install": saved["auto_install"] == json!(true),
                "shell": shell, "notify_sound": notify_sound, "notify_mode": notify_mode}),
            )
        }
        "settings.set" => {
            // partial update: only the keys present are changed
            let mut saved = read_settings(&reg.saved);
            if let Some(level) = p["terse"].as_str() {
                if !["off", "short", "tiny"].contains(&level) {
                    return Err(anyhow::anyhow!("terse must be off, short or tiny"));
                }
                saved["terse"] = json!(level);
            }
            if p.get("pm").is_some() {
                saved["pm"] = match p["pm"].as_str() {
                    Some(m) if ["bun", "pnpm", "yarn", "npm"].contains(&m) => json!(m),
                    Some(_) => return Err(anyhow::anyhow!("pm must be bun, pnpm, yarn or npm")),
                    None => Value::Null,
                };
            }
            if let Some(b) = p["auto_install"].as_bool() {
                saved["auto_install"] = json!(b);
            }
            apply_ui_settings(&mut saved, &p)?;
            write_atomic(&reg.saved.with_file_name("settings.json"), &serde_json::to_vec_pretty(&saved)?)?;
            Ok(json!({}))
        }
        "dispatch.meta" => {
            // titles and archive flags the dispatcher keeps per worktree (see `axelstra dispatch`)
            let f = reg.saved.with_file_name("dispatch.json");
            Ok(std::fs::read_to_string(f).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()).unwrap_or_else(|| json!({})))
        }
        "session.pending" => {
            let sid_dir = reg.saved.with_file_name("sid");
            let mut v = reg.pending.lock_recover().clone();
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
                    // only a conversation file that exists and has no replies counts as empty; a file that cannot be found
                    // (another config dir, a different path spelling) is resumed and, failing that, started fresh
                    let empty = crate::agentlog::claude_usage(cwd, Some(&id)).map(|u| u.messages == 0).unwrap_or(false);
                    if empty {
                        rec["meta"]["blank"] = Value::Bool(true);
                    }
                    rec["meta"]["resume"] = Value::String(id);
                }
            }
            Ok(Value::Array(v))
        }
        "session.pending_remove" => {
            // one restored (or deliberately skipped) entry leaves the list; the rest wait, so a failure midway loses nothing
            let rec = &p["rec"];
            let same = |a: &Value| match (a["meta"]["key"].as_str(), rec["meta"]["key"].as_str()) {
                (Some(x), Some(y)) => x == y,
                _ => a["program"] == rec["program"] && a["cwd"] == rec["cwd"] && a["label"] == rec["label"],
            };
            let mut list = reg.pending.lock_recover();
            if let Some(i) = list.iter().position(same) {
                list.remove(i);
            }
            drop(list);
            reg.persist();
            Ok(json!({}))
        }
        "session.pending_clear" => {
            reg.pending.lock_recover().clear();
            reg.persist();
            Ok(json!({}))
        }
        "session.kill" => {
            session()?.kill();
            Ok(json!({}))
        }
        // the last lines as the terminal shows them (replayed through an emulator), for `axelstra read` and flow reports
        "session.screen" => {
            let s = session()?;
            let n = p["lines"].as_u64().unwrap_or(40).clamp(1, 500) as usize;
            Ok(json!({"lines": tokio::task::block_in_place(|| s.screen_lines(n))}))
        }
        "session.resources" => {
            // memory now and total CPU time of every live session (the caller turns CPU time into a percentage)
            let l = reg
                .sessions
                .lock_recover()
                .iter()
                .filter(|(_, s)| s.try_wait().is_none())
                .map(|(id, s)| (*id, s.clone()))
                .collect::<Vec<_>>();
            let v: Vec<Value> = l.iter().map(|(id, s)| json!({"id": id, "mem": s.memory_now(), "cpu_100ns": s.job_usage().1})).collect();
            Ok(json!({"sessions": v, "cores": std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1)}))
        }
        "session.usage" => {
            let (mem, cpu) = session()?.job_usage();
            Ok(json!({"peak_mem_bytes": mem, "cpu_100ns": cpu}))
        }
        "worktree.status" => {
            // only paths the daemon knows; the list comes from the UI and git runs in each
            let list: Vec<String> =
                p["paths"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default();
            let norm = |s: &str| s.replace('\\', "/").to_ascii_lowercase();
            let out = tokio::task::block_in_place(|| {
                let mut allowed = std::collections::HashSet::new();
                for pr in crate::projects::load(&reg.store) {
                    for w in axelstra_core::git::worktree_list(std::path::Path::new(&pr.path)).unwrap_or_default() {
                        allowed.insert(norm(&w.path));
                    }
                }
                list.iter().take(64).filter(|w| allowed.contains(&norm(w))).map(|w| crate::wtstatus::status_cached(w)).collect::<Vec<_>>()
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
        "agent.worktree_usage" => {
            let paths: Vec<String> =
                p["worktrees"].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect()).unwrap_or_default();
            let usage = tokio::task::block_in_place(|| crate::agentlog::worktree_usage(&paths));
            Ok(json!({"usage": usage}))
        }
        "project.list" => {
            if let Some((at, v)) = reg.plist.lock_recover().as_ref() {
                if at.elapsed() < Duration::from_secs(3) {
                    return Ok(v.clone());
                }
            }
            let ps = crate::projects::load(&reg.store);
            let v: Vec<Value> = ps
                .iter()
                .map(|pr| {
                    let repo = std::path::Path::new(&pr.path);
                    let mut wts = axelstra_core::git::worktree_list(repo).unwrap_or_default();
                    wts.retain(|w| w.is_main || w.path.replace('\\', "/").contains("/.axelstra-wt/"));
                    let ident = axelstra_core::git::identity(repo);
                    json!({"id": pr.id, "name": ident.display_name, "path": pr.path, "worktrees": wts, "git": axelstra_core::git::is_repo(repo),
                           "setup": pr.setup, "default_branch": axelstra_core::git::default_branch(repo), "slug": ident.slug, "icon_url": ident.icon_url, "remote": ident.remote})
                })
                .collect();
            let all = Value::Array(v);
            *reg.plist.lock_recover() = Some((std::time::Instant::now(), all.clone()));
            Ok(all)
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
            if reg.runs.lock_recover().get(&pr.id).is_some_and(|r| r["state"] == "running") {
                anyhow::bail!("an orchestration is already running for this project");
            }
            let repo = axelstra_core::git::canon(std::path::Path::new(&pr.path))?;
            let name = format!("ui-{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs());
            let max = p["max"].as_u64().unwrap_or(3).clamp(1, 8) as usize;
            let trust = p["trust_acceptance"].as_bool().unwrap_or(false);
            // overriding the agent command is for tests and custom agents; the same
            // client can already spawn any process, so this grants nothing new
            let agent: Option<Vec<String>> = p["agent"].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect());
            // connects back to this same daemon: must not hold an async worker
            let runner = tokio::task::block_in_place(|| crate::runner::DaemonRunner::connect(&reg.pipe, &reg.token))?;
            let started = json!({"state": "running", "name": name, "integ_branch": format!("orch/{name}")});
            reg.runs.lock_recover().insert(pr.id.clone(), started.clone());
            let (reg2, pid, n2) = (reg.clone(), pr.id.clone(), name.clone());
            std::thread::spawn(move || {
                let opts = axelstra_core::orch::Options {
                    wt_root: repo.parent().unwrap_or(&repo).join(".axelstra-wt"),
                    repo,
                    name: n2.clone(),
                    max_concurrency: max,
                    agent_override: agent,
                    poll_ms: 500,
                    runner: Some(Arc::new(runner)),
                };
                // the UI sets this only after the user confirmed the acceptance commands
                let res = if trust { axelstra_core::orch::run_trusted(&opts) } else { axelstra_core::orch::run(&opts) };
                let v = match res {
                    Ok(r) => json!({"state": "done", "name": n2, "integ_branch": r.integ_branch,
                        "integ_wt": r.integ_wt, "merged": r.merged, "failed": r.failed,
                        "needs_review": r.needs_review, "blocked": r.blocked}),
                    Err(e) => json!({"state": "error", "name": n2, "error": format!("{e:#}")}),
                };
                reg2.runs.lock_recover().insert(pid, v);
            });
            Ok(started)
        }
        "orch.retry" => {
            let pr = crate::projects::find(&reg.store, p["project"].as_str().unwrap_or_default())?;
            let id = p["id"].as_str().unwrap_or_default().to_string();
            let name = reg.runs.lock().unwrap().get(&pr.id).and_then(|r| r["name"].as_str().map(String::from));
            let name = name.ok_or_else(|| anyhow::anyhow!("no orchestration run to retry"))?;
            let trust = p["trust_acceptance"].as_bool().unwrap_or(false);
            let repo = axelstra_core::git::canon(std::path::Path::new(&pr.path))?;
            let wt_root = repo.parent().unwrap_or(&repo).join(".axelstra-wt");
            tokio::task::block_in_place(|| axelstra_core::orch::retry_merge(&repo, &wt_root, &name, &id, trust))?;
            if let Some(r) = reg.runs.lock().unwrap().get_mut(&pr.id) {
                if let Some(v) = r["needs_review"].as_array_mut() {
                    v.retain(|e| e[0] != id.as_str());
                }
            }
            Ok(json!({}))
        }
        "orch.status" => {
            let id = p["project"].as_str().unwrap_or_default();
            Ok(reg.runs.lock_recover().get(id).cloned().unwrap_or(json!({"state": "idle"})))
        }
        "kanban.list" | "kanban.save" | "kanban.move" | "kanban.delete" => {
            let pr = crate::projects::find(&reg.store, p["project"].as_str().unwrap_or_default())?;
            let repo = std::path::PathBuf::from(&pr.path);
            let s = |k: &str| p[k].as_str().unwrap_or_default().to_string();
            tokio::task::block_in_place(|| -> Result<Value> {
                use crate::kanban;
                Ok(match req.method.as_str() {
                    "kanban.list" => serde_json::to_value(kanban::list(&repo)?)?,
                    "kanban.save" => {
                        serde_json::to_value(kanban::save(&repo, p["id"].as_str(), &s("title"), &s("body"), p["status"].as_str(), &p)?)?
                    }
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
        "git.commit" | "git.push" | "git.pr" | "git.suggest" => {
            let wt = crate::projects::known_worktree(&reg.store, p["worktree"].as_str().unwrap_or_default())?;
            let s = |k: &str| p[k].as_str().unwrap_or_default().to_string();
            tokio::task::block_in_place(|| -> Result<Value> {
                use crate::publish;
                Ok(match req.method.as_str() {
                    "git.commit" => serde_json::to_value(publish::commit(&wt, &s("message"))?)?,
                    "git.push" => json!({"output": publish::push(&wt)?}),
                    "git.suggest" => json!({"message": publish::suggest_message(&wt)?}),
                    _ => json!({"url": publish::pull_request(&wt, &s("title"), &s("body"), p["base"].as_str())?}),
                })
            })
        }
        "docker.open_logs" | "docker.open_shell" => {
            let (s, label, meta) = crate::docker::terminal(&reg.store, &req.method, p)?;
            Ok(json!({"id": reg.spawn(&s, Some(&label), Some(meta))?}))
        }
        "docker.status" | "docker.containers" | "docker.stats" | "docker.images" | "docker.volumes" | "docker.networks"
        | "docker.inspect" | "docker.logs" | "docker.fs_list" | "docker.fs_read" | "docker.action" => {
            tokio::task::block_in_place(|| crate::docker::call(&reg.store, &req.method, p))
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
            ensure_session_notifier(reg, tx, subs);
            let wt = crate::projects::known_worktree(&reg.store, p["worktree"].as_str().unwrap_or_default())?;
            if let Some(old) = subs.remove(&u32::MAX) {
                old.store(true, Ordering::SeqCst);
            }
            let stop = Arc::new(AtomicBool::new(false));
            let (tx2, path) = (tx.clone(), wt.to_string_lossy().to_string());
            tokio::task::block_in_place(|| {
                crate::watcher::start(wt, stop.clone(), move || {
                    let ev = json!({"event": "fs.changed", "worktree": path});
                    let _ = tx2.blocking_send(encode(EVENT, ev.to_string().as_bytes()));
                })
            })?;
            subs.insert(u32::MAX, stop);
            Ok(json!({}))
        }
        "watch.unsubscribe" => {
            ensure_session_notifier(reg, tx, subs);
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
        "pm.detect" => {
            let path =
                p["id"].as_str().and_then(|id| crate::projects::find(&reg.store, id).ok()).map(|pr| std::path::PathBuf::from(pr.path));
            let preferred = read_settings(&reg.saved)["pm"].as_str().map(String::from);
            let (installed, suggested) =
                tokio::task::block_in_place(|| crate::projects::package_managers(path.as_deref(), preferred.as_deref()));
            let has_package_json = path.as_ref().map(|p| p.join("package.json").is_file()).unwrap_or(false);
            Ok(json!({"installed": installed, "suggested": suggested, "package_json": has_package_json}))
        }
        "project.remove" => {
            crate::projects::remove(&reg.store, p["id"].as_str().unwrap_or_default())?;
            Ok(json!({}))
        }
        "worktree.create" => {
            let pr = crate::projects::find(&reg.store, p["project"].as_str().unwrap_or_default())?;
            let name = p["name"].as_str().ok_or_else(|| anyhow::anyhow!("missing name"))?;
            let (wt, env) = tokio::task::block_in_place(|| crate::projects::worktree_create_report(&pr, name))?;
            // the worktree plus what .worktreeinclude copied (env_copied) and left out (env_skipped)
            let mut v = serde_json::to_value(wt)?;
            v["env_copied"] = json!(env.copied);
            v["env_skipped"] = json!(env.skipped);
            Ok(v)
        }
        "worktree.rename" => {
            let pr = crate::projects::find(&reg.store, p["project"].as_str().unwrap_or_default())?;
            let path = p["path"].as_str().ok_or_else(|| anyhow::anyhow!("missing path"))?;
            let name = p["name"].as_str().ok_or_else(|| anyhow::anyhow!("missing name"))?;
            let busy = reg.worktree_working(path);
            let wt = tokio::task::block_in_place(|| crate::projects::worktree_rename(&pr, path, name, busy))?;
            // the dispatcher's notes (title, requests) are keyed by path: they follow the folder
            let f = reg.saved.with_file_name("dispatch.json");
            let norm = |s: &str| s.replace('\\', "/").trim_end_matches('/').to_ascii_lowercase();
            if let Some(mut meta) = std::fs::read_to_string(&f).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()) {
                if let Some(e) = meta.as_object_mut().and_then(|m| m.remove(&norm(path))) {
                    meta[norm(&wt.path)] = e;
                    write_atomic(&f, &serde_json::to_vec_pretty(&meta)?)?;
                }
            }
            Ok(serde_json::to_value(wt)?)
        }
        "project.env_preview" => {
            // what .worktreeinclude would copy into the next worktree, for the project settings dialog
            let pr = crate::projects::find(&reg.store, p["project"].as_str().unwrap_or_default())?;
            Ok(serde_json::to_value(tokio::task::block_in_place(|| crate::wtenv::plan(std::path::Path::new(&pr.path)))?)?)
        }
        "dispatch.overlaps" => {
            let f = reg.saved.with_file_name("dispatch.json");
            Ok(tokio::task::block_in_place(|| crate::overlaps::overlaps(&reg.store, &f)))
        }
        "ports.list" => {
            let wt = crate::projects::known_worktree(&reg.store, p["worktree"].as_str().unwrap_or_default())?;
            let norm = |s: &str| s.replace('\\', "/").to_ascii_lowercase();
            let key = norm(&wt.to_string_lossy());
            let ids: Vec<u32> = {
                let metas = reg.metas.lock_recover();
                metas.iter().filter(|(_, m)| m["worktree"].as_str().map(|w| norm(w)) == Some(key.clone())).map(|(id, _)| *id).collect()
            };
            let pids: Vec<u32> =
                ids.iter().filter_map(|id| reg.get(*id)).filter(|s| s.try_wait().is_none()).filter_map(|s| s.pid()).collect();
            Ok(json!({"ports": tokio::task::block_in_place(|| crate::ports::listening(&pids))?}))
        }
        "automation.list" => {
            let _g = reg.auto_lock.lock_recover();
            Ok(json!({"automations": crate::automations::load(&crate::automations::store_path(&reg.store))}))
        }
        "automation.save" => {
            let _g = reg.auto_lock.lock_recover();
            let path = crate::automations::store_path(&reg.store);
            let mut a: crate::automations::Automation =
                serde_json::from_value(p.clone()).map_err(|e| anyhow::anyhow!("invalid automation: {e}"))?;
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
            let _g = reg.auto_lock.lock_recover();
            let path = crate::automations::store_path(&reg.store);
            let mut list = crate::automations::load(&path);
            list.retain(|x| x.id != p["id"].as_str().unwrap_or_default());
            crate::automations::save_all(&path, &list)?;
            Ok(json!({}))
        }
        "automation.run" => {
            let _g = reg.auto_lock.lock_recover();
            let path = crate::automations::store_path(&reg.store);
            tokio::task::block_in_place(|| run_automation(reg, &path, p["id"].as_str().unwrap_or_default()))
        }
        "open.editor" => {
            let wt = crate::projects::known_worktree(&reg.store, p["worktree"].as_str().unwrap_or_default())?;
            // only editors we know; never a program name from the client, and never through the shell
            let (prog, args) = match p["app"].as_str().unwrap_or_default() {
                "code" => crate::fsops::code_launch(&wt)?,
                other => anyhow::bail!("unknown editor {other:?}"),
            };
            axelstra_core::git::quiet(&mut std::process::Command::new(&prog))
                .args(args)
                .spawn()
                .map_err(|e| anyhow::anyhow!("could not start {}: {e}", prog.display()))?;
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
            // remember what was going on, so the next start can pick it up: agents that were working, and the
            // ephemeral workers that die with this daemon (they are not restored) per coordinator
            let live: Vec<(u32, bool)> = reg
                .sessions
                .lock_recover()
                .iter()
                .filter(|(_, s)| s.try_wait().is_none())
                .map(|(id, s)| (*id, s.idle_ms() < 5000))
                .collect();
            let mut working: Vec<u32> = vec![];
            let mut lost: HashMap<u32, Vec<String>> = HashMap::new();
            {
                let metas = reg.metas.lock_recover();
                for (id, busy) in &live {
                    let m = metas.get(id);
                    if reg.bus.is_reaped(*id) {
                        if let Some(parent) = reg.bus.parent_of(*id) {
                            let wt = m.and_then(|m| m["worktree"].as_str()).unwrap_or("?");
                            lost.entry(parent).or_default().push(wt.to_string());
                        }
                    } else if *busy && m.and_then(|m| m["kind"].as_str()) == Some("agent") {
                        working.push(*id);
                    }
                }
            }
            {
                let mut specs = reg.specs.lock_recover();
                for (id, spec) in specs.iter_mut() {
                    if working.contains(id) {
                        spec["meta"]["interrupted"] = Value::Bool(true);
                    }
                    if let Some(w) = lost.get(id) {
                        spec["meta"]["lost_workers"] = json!(w);
                    }
                }
            }
            reg.persist();
            // keep what each agent terminal showed, so the restored tab is not empty
            save_histories(reg);
            // end every terminal and agent now (their specs stay in sessions.json, so they restore on the next start)
            for s in reg.sessions.lock_recover().values() {
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

#[cfg(test)]
mod settings_tests {
    use super::*;

    #[test]
    fn ui_settings_default_when_missing_or_invalid() {
        assert_eq!(ui_settings(&json!({})), ("auto", true, "all"));
        assert_eq!(ui_settings(&json!({"shell": "zsh", "notify_mode": "x", "notify_sound": "no"})), ("auto", true, "all"));
    }

    #[test]
    fn ui_settings_patch_persists_valid_keys_and_leaves_others() {
        let mut saved = json!({"terse": "short"});
        apply_ui_settings(&mut saved, &json!({"shell": "wsl", "notify_sound": false, "notify_mode": "waiting"})).unwrap();
        assert_eq!(ui_settings(&saved), ("wsl", false, "waiting"));
        assert_eq!(saved["terse"], "short");
        // a patch without these keys changes nothing
        apply_ui_settings(&mut saved, &json!({"terse": "off"})).unwrap();
        assert_eq!(ui_settings(&saved), ("wsl", false, "waiting"));
    }

    #[test]
    fn ui_settings_patch_rejects_invalid_values() {
        let mut saved = json!({});
        assert!(apply_ui_settings(&mut saved, &json!({"shell": "zsh"})).is_err());
        assert!(apply_ui_settings(&mut saved, &json!({"notify_mode": "loud"})).is_err());
        assert_eq!(saved, json!({}));
    }
}

#[cfg(test)]
mod registry_tests {
    use super::*;

    #[test]
    fn within_compares_whole_path_components() {
        assert!(within("c:/a/wt", "c:/a/wt"));
        assert!(within("c:/a/wt/sub/dir", "c:/a/wt"));
        assert!(!within("c:/a/wt-2", "c:/a/wt"));
        assert!(!within("c:/a/wtx/sub", "c:/a/wt"));
    }

    fn temp_registry(tag: &str) -> Arc<Registry> {
        let dir = std::env::temp_dir().join(format!("axl-reg-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        Arc::new(Registry::new(dir.join("projects.json"), "p".into(), "t".into()))
    }

    #[test]
    fn corrupt_sessions_file_is_kept_aside_and_not_overwritten() {
        let dir = std::env::temp_dir().join(format!("axl-reg-bad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("sessions.json"), "[{ broken").unwrap();
        let reg = Registry::new(dir.join("projects.json"), "p".into(), "t".into());
        assert!(reg.pending.lock_recover().is_empty());
        reg.persist();
        assert_eq!(std::fs::read_to_string(dir.join("sessions.json.bad")).unwrap(), "[{ broken");
        assert_eq!(std::fs::read_to_string(dir.join("sessions.json")).unwrap(), "[]");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn registry_survives_a_poisoned_lock() {
        let reg = temp_registry("poison");
        let r = reg.clone();
        let _ = std::thread::spawn(move || {
            let _g = r.metas.lock().unwrap();
            panic!("handler bug");
        })
        .join();
        assert!(reg.metas.is_poisoned());
        assert!(session_list(&reg).is_empty());
        save_histories(&reg);
    }

    /// `session.list` and `daemon.shutdown` both need sessions and metas: with one lock order they never wait on each other.
    #[test]
    fn lock_order_paths_do_not_deadlock() {
        let reg = temp_registry("order");
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        for i in 0..4 {
            let (r, d) = (reg.clone(), done_tx.clone());
            std::thread::spawn(move || {
                for _ in 0..2000 {
                    if i % 2 == 0 {
                        session_list(&r);
                    } else {
                        save_histories(&r);
                        r.persist();
                    }
                }
                let _ = d.send(());
            });
        }
        for _ in 0..4 {
            done_rx.recv_timeout(Duration::from_secs(30)).expect("deadlock between registry locks");
        }
    }
}
