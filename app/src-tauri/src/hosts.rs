//! Remote hosts (M7b): a registry of named daemons (`wsl:<distro>`, `ssh:[user@]host[:port]`) and the routing of the
//! webview's calls to the daemon that owns the project. See docs/remote-daemon.md.
//!
//! The page sees one flat world: a remote project's paths and project id carry the prefix `@<host>:` and a remote
//! session id carries the host slot in its top byte (`slot << 24 | id`). This module strips those before a call leaves
//! for the host and adds them to what comes back, so every daemon keeps its own plain paths and ids and the frontend
//! never has two different things under one key. Local paths and ids are untouched.
//!
//! The pure part (prefixes, ids, the JSON rewrite, the routing plan) has no I/O and is unit tested. The rest holds one
//! `Remote` per registered host: three lazily opened control connections (control, bg, input), one event connection
//! and one connection per attached terminal, each of them its own `ssh`/`wsl.exe` child speaking to `axelstra bridge`.

use anyhow::{Context, Result};
use axelstra_core::msg::Coded;
use axelstra_daemon::client::{Client, Frame};
use axelstra_daemon::remote::RemoteSpec;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};
use tauri::ipc::{Channel, InvokeResponseBody};

pub const LOCAL: &str = "local";
/// Highest slot: ids above `1 << 24` do not exist (the daemon counts sessions from 1), so the top byte is free, and 127
/// keeps the whole id under 2^31.
pub const MAX_SLOT: u32 = 127;
/// Most hosts the registry holds (the daemon enforces the same).
pub const MAX_HOSTS: usize = 16;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

// ---------------------------------------------------------------------------------------------------------------------
// pure: ids and prefixes

pub fn gid(slot: u32, id: u32) -> u32 {
    (slot << 24) | (id & 0x00FF_FFFF)
}

/// `(slot, id on that host)`; slot 0 is this machine.
pub fn split_gid(g: u32) -> (u32, u32) {
    (g >> 24, g & 0x00FF_FFFF)
}

pub fn valid_id(id: &str) -> bool {
    let b = id.as_bytes();
    !b.is_empty()
        && b.len() <= 24
        && b[0].is_ascii_lowercase()
        && b.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
}

pub fn tag(host: &str, s: &str) -> String {
    format!("@{host}:{s}")
}

/// `@<host>:<rest>` -> `(host, rest)` when the host part is a well-formed id.
pub fn untag(s: &str) -> Option<(&str, &str)> {
    let r = s.strip_prefix('@')?;
    let (host, rest) = r.split_once(':')?;
    valid_id(host).then_some((host, rest))
}

/// An absolute path in either flavour (`/x`, `C:\x`, `C:/x`, `\\srv\x`).
fn looks_absolute(s: &str) -> bool {
    let b = s.as_bytes();
    s.starts_with('/')
        || s.starts_with("\\\\")
        || (b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && matches!(b[2], b'/' | b'\\'))
}

/// Keys whose string values are paths (or project ids) that name a place on one host.
const PATH_KEYS: &[&str] = &["worktree", "worktrees", "project", "cwd", "root", "main", "parent", "repo", "dir", "path", "paths", "id"];
/// Keys whose number values are session ids (`id` only inside `session.*`).
const SESSION_KEYS: &[&str] = &["session", "parent", "sessions"];

/// Fan-out: no key says where, the answer is every connected host's.
const FAN: &[&str] = &["project.list", "session.list", "session.pending"];
/// A list of paths that may belong to several hosts: split, ask each, join the arrays of the answers.
const SPLIT: &[(&str, &str)] = &[("worktree.status", "paths"), ("agent.worktree_usage", "worktrees"), ("budget.status", "worktrees")];
/// Needs the machine the app runs on (its Docker, its screen, its editor, its automations): refused for a remote project.
const LOCAL_ONLY: &[&str] = &["docker", "computer", "open", "automation", "hooks", "backup", "workspace", "app", "daemon", "settings"];

#[derive(Debug, PartialEq)]
pub enum Plan {
    Local,
    /// the call goes to this host with these (already stripped) params
    Remote(String, Value),
    /// several hosts: `(host or LOCAL, params)` each, the arrays of the answers joined
    Split(Vec<(String, Value)>),
    Fan,
}

struct Walk<'a> {
    method: &'a str,
    slot_host: &'a dyn Fn(u32) -> Option<String>,
    hosts: BTreeSet<String>,
    err: Option<Coded>,
}

impl Walk<'_> {
    fn strip(&mut self, v: &mut Value, key: Option<&str>) {
        match v {
            Value::Object(m) => {
                for (k, x) in m.iter_mut() {
                    self.strip(x, Some(k));
                }
            }
            Value::Array(a) => {
                for x in a.iter_mut() {
                    self.strip(x, key);
                }
            }
            Value::String(s) => {
                if key.is_some_and(|k| PATH_KEYS.contains(&k)) {
                    if let Some((h, rest)) = untag(s) {
                        self.hosts.insert(h.to_string());
                        *s = rest.to_string();
                    }
                }
            }
            Value::Number(n) => {
                let session_key = key.is_some_and(|k| SESSION_KEYS.contains(&k) || (k == "id" && self.method.starts_with("session.")));
                if let (true, Some(g)) = (session_key, n.as_u64().and_then(|g| u32::try_from(g).ok())) {
                    let (slot, raw) = split_gid(g);
                    if slot != 0 {
                        match (self.slot_host)(slot) {
                            Some(h) => {
                                self.hosts.insert(h);
                                *v = json!(raw);
                            }
                            None => {
                                self.err = Some(Coded::new(
                                    "host.stale_session",
                                    format!("session {g} belongs to a host that no longer exists"),
                                    &[("id", g.to_string())],
                                ))
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

fn host_known(h: &str, known: &dyn Fn(&str) -> bool) -> Result<(), Coded> {
    if h == LOCAL || known(h) {
        Ok(())
    } else {
        Err(Coded::new("host.unknown", format!("unknown host: {h}"), &[("host", h.to_string())]))
    }
}

/// Decides which host a call is for and rewrites its params for that host.
pub fn plan(method: &str, params: &Value, slot_host: &dyn Fn(u32) -> Option<String>, known: &dyn Fn(&str) -> bool) -> Result<Plan, Coded> {
    if FAN.contains(&method) {
        return Ok(Plan::Fan);
    }
    // the host named outright (browsing a folder to add as a project)
    if method.starts_with("host.") {
        let h = params["host"].as_str().unwrap_or(LOCAL);
        if h == LOCAL {
            return Ok(Plan::Local);
        }
        host_known(h, known)?;
        let mut p = params.clone();
        if let Some(o) = p.as_object_mut() {
            o.remove("host");
        }
        return Ok(Plan::Remote(h.to_string(), p));
    }
    if let Some((_, key)) = SPLIT.iter().find(|(m, _)| *m == method) {
        let items = params[*key].as_array().cloned().unwrap_or_default();
        let mut groups: BTreeMap<String, Vec<Value>> = BTreeMap::new();
        for it in items {
            let s = it.as_str().unwrap_or_default();
            match untag(s) {
                Some((h, rest)) => {
                    host_known(h, known)?;
                    groups.entry(h.to_string()).or_default().push(json!(rest));
                }
                None => groups.entry(LOCAL.to_string()).or_default().push(it),
            }
        }
        if groups.keys().all(|h| h == LOCAL) {
            return Ok(Plan::Local);
        }
        let calls = groups
            .into_iter()
            .map(|(h, list)| {
                let mut p = params.clone();
                p[*key] = Value::Array(list);
                (h, p)
            })
            .collect();
        return Ok(Plan::Split(calls));
    }
    let mut p = params.clone();
    let mut w = Walk { method, slot_host, hosts: BTreeSet::new(), err: None };
    w.strip(&mut p, None);
    if let Some(e) = w.err {
        return Err(e);
    }
    let mut hosts = w.hosts.into_iter();
    let Some(first) = hosts.next() else { return Ok(Plan::Local) };
    if hosts.next().is_some() {
        return Err(Coded::new("host.mixed", "the request mixes different hosts".into(), &[]));
    }
    host_known(&first, known)?;
    let family = method.split('.').next().unwrap_or_default();
    if LOCAL_ONLY.contains(&family) {
        return Err(Coded::new(
            "host.remote_unsupported",
            format!("{method} is not supported on remote projects"),
            &[("method", method.to_string())],
        ));
    }
    Ok(Plan::Remote(first, p))
}

/// Adds the host's prefix and slot to a result or event of `method` that came from `host`.
pub fn tag_result(method: &str, host: &str, slot: u32, v: &mut Value) {
    fn walk(m: &str, host: &str, slot: u32, v: &mut Value, key: Option<&str>) {
        match v {
            Value::Object(o) => {
                for (k, x) in o.iter_mut() {
                    walk(m, host, slot, x, Some(k));
                }
            }
            Value::Array(a) => {
                for x in a.iter_mut() {
                    walk(m, host, slot, x, key);
                }
            }
            Value::String(s) => {
                let Some(k) = key else { return };
                if !PATH_KEYS.contains(&k) || s.is_empty() || untag(s).is_some() {
                    return;
                }
                // `project`/`worktree` are always places (or ids); the other keys also hold relative paths (fs.list)
                let place = matches!(k, "project" | "worktree" | "worktrees") || looks_absolute(s);
                if place && k != "id" {
                    *s = tag(host, s);
                }
            }
            Value::Number(n) => {
                let session_key = key.is_some_and(|k| SESSION_KEYS.contains(&k) || (k == "id" && m.starts_with("session.")));
                if let (true, Some(id)) = (session_key, n.as_u64().and_then(|g| u32::try_from(g).ok())) {
                    if id < (1 << 24) {
                        *v = json!(gid(slot, id));
                    }
                }
            }
            _ => {}
        }
    }
    walk(method, host, slot, v, None);
}

/// `project.list` / `project.add` entries: the id is also made global.
pub fn tag_project(host: &str, slot: u32, p: &mut Value) {
    let id = p["id"].as_str().map(|s| tag(host, s));
    tag_result("project.list", host, slot, p);
    if let Some(id) = id {
        p["id"] = json!(id);
    }
    p["host"] = json!(host);
}

/// Joins the answers of a split call: arrays under the same key are concatenated, other keys keep the first answer.
pub fn join_answers(answers: Vec<Value>) -> Value {
    let mut out = json!({});
    for a in answers {
        let Some(o) = a.as_object() else { continue };
        for (k, v) in o {
            match (out.get_mut(k), v) {
                (Some(Value::Array(dst)), Value::Array(src)) => dst.extend(src.iter().cloned()),
                (None, _) => out[k] = v.clone(),
                _ => {}
            }
        }
    }
    out
}

/// A name turned into an id: lowercase letters and digits, single dashes, starts with a letter, at most 24 characters.
pub fn slug(name: &str, taken: &dyn Fn(&str) -> bool) -> String {
    let mut s = String::new();
    for c in name.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            s.push(c);
        } else if !s.ends_with('-') && !s.is_empty() {
            s.push('-');
        }
    }
    let s = s.trim_matches('-');
    let mut s = if s.chars().next().is_some_and(|c| c.is_ascii_lowercase()) { s.to_string() } else { format!("h{s}") };
    s.truncate(24);
    let base = s.trim_end_matches('-').to_string();
    let mut id = base.clone();
    let mut n = 2;
    while id == LOCAL || taken(&id) {
        let suffix = format!("-{n}");
        id = format!("{}{suffix}", &base[..base.len().min(24 - suffix.len())]);
        n += 1;
    }
    id
}

// ---------------------------------------------------------------------------------------------------------------------
// the registry and the connections

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum St {
    Idle,
    Connecting,
    Ok,
    Lost,
    Auth,
}

impl St {
    pub fn name(self) -> &'static str {
        match self {
            St::Idle => "idle",
            St::Connecting => "connecting",
            St::Ok => "ok",
            St::Lost => "lost",
            St::Auth => "auth",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Def {
    pub id: String,
    pub name: String,
    pub spec: String,
    pub slot: u32,
    pub bin: Option<String>,
}

impl Def {
    fn from_json(v: &Value) -> Option<Def> {
        Some(Def {
            id: v["id"].as_str().filter(|s| valid_id(s))?.to_string(),
            name: v["name"].as_str()?.to_string(),
            spec: v["spec"].as_str()?.to_string(),
            slot: v["slot"].as_u64().filter(|s| (1..=u64::from(MAX_SLOT)).contains(s))? as u32,
            bin: v["bin"].as_str().map(String::from),
        })
    }

    fn to_json(&self) -> Value {
        let mut v = json!({"id": self.id, "name": self.name, "spec": self.spec, "slot": self.slot});
        if let Some(b) = &self.bin {
            v["bin"] = json!(b);
        }
        v
    }

    pub fn remote_spec(&self) -> Result<RemoteSpec> {
        let s = RemoteSpec::parse(&self.spec)?;
        match &self.bin {
            Some(b) => s.with_bin(b),
            None => Ok(s),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Which {
    Control,
    Bg,
    Input,
}

struct Info {
    state: St,
    error: Option<String>,
    /// the code of `error` when it is a single coded message (no tool output appended)
    error_code: Option<Coded>,
    proto: Option<u64>,
    build: Option<String>,
    next_try: Instant,
    fails: u32,
}

pub struct Remote {
    pub def: Def,
    info: Mutex<Info>,
    /// tells the keeper thread that `info` changed (state, next try) or that the host was removed
    wake: Condvar,
    control: Mutex<Option<Client>>,
    bg: Mutex<Option<Client>>,
    input: Mutex<Option<Client>>,
    /// write side of the event connection (`watch.subscribe`)
    watch: Mutex<Option<Client>>,
    /// one connect at a time per host
    gate: Mutex<()>,
    instance: AtomicU64,
    /// bumped whenever the host is marked lost: an event reader of an older link ends quietly
    link: AtomicU64,
}

static HOSTS: Mutex<Vec<Arc<Remote>>> = Mutex::new(Vec::new());
static EVENTS: Mutex<Option<Channel<InvokeResponseBody>>> = Mutex::new(None);
/// Host whose worktree the event stream was last pointed at (None = this machine).
static WATCHING: Mutex<Option<String>> = Mutex::new(None);

pub fn get(id: &str) -> Option<Arc<Remote>> {
    lock(&HOSTS).iter().find(|h| h.def.id == id).cloned()
}

pub fn by_slot(slot: u32) -> Option<Arc<Remote>> {
    lock(&HOSTS).iter().find(|h| h.def.slot == slot).cloned()
}

pub fn all() -> Vec<Arc<Remote>> {
    lock(&HOSTS).clone()
}

fn known(id: &str) -> bool {
    get(id).is_some()
}

fn slot_host(slot: u32) -> Option<String> {
    by_slot(slot).map(|h| h.def.id.clone())
}

fn emit(name: &str, payload: Value) {
    use tauri::Emitter;
    if let Some(app) = crate::APP.get() {
        let _ = app.emit(name, payload);
    }
}

impl Remote {
    fn new(def: Def) -> Arc<Remote> {
        Arc::new(Remote {
            def,
            info: Mutex::new(Info {
                state: St::Idle,
                error: None,
                error_code: None,
                proto: None,
                build: None,
                next_try: Instant::now(),
                fails: 0,
            }),
            wake: Condvar::new(),
            control: Mutex::new(None),
            bg: Mutex::new(None),
            input: Mutex::new(None),
            watch: Mutex::new(None),
            gate: Mutex::new(()),
            instance: AtomicU64::new(0),
            link: AtomicU64::new(0),
        })
    }

    pub fn state(&self) -> St {
        lock(&self.info).state
    }

    pub fn describe(&self) -> Value {
        let i = lock(&self.info);
        let mut v = self.def.to_json();
        v["state"] = json!(i.state.name());
        if let Some(e) = &i.error {
            v["error"] = json!(e);
            if let Some(c) = &i.error_code {
                v["error_code"] = json!(c.code);
                v["error_params"] = c.params_json();
            }
        }
        if let Some(p) = i.proto {
            v["proto"] = json!(p);
        }
        if let Some(b) = &i.build {
            v["build"] = json!(b);
        }
        v
    }

    fn slot(&self, which: Which) -> &Mutex<Option<Client>> {
        match which {
            Which::Control => &self.control,
            Which::Bg => &self.bg,
            Which::Input => &self.input,
        }
    }

    fn set_state(&self, state: St, error: Option<String>) {
        self.set_state_coded(state, error, None);
    }

    fn set_state_coded(&self, state: St, error: Option<String>, code: Option<Coded>) {
        {
            let mut i = lock(&self.info);
            if i.state == state && i.error == error {
                return;
            }
            i.state = state;
            i.error = error.clone();
            i.error_code = code.clone();
        }
        self.wake.notify_all();
        let mut p = json!({"id": self.def.id, "state": state.name()});
        if let Some(e) = error {
            p["error"] = json!(e);
            if let Some(c) = code {
                p["error_code"] = json!(c.code);
                p["error_params"] = c.params_json();
            }
        }
        emit("host-state", p);
    }

    /// The link is gone (or never came up): every connection is dropped (which kills the child processes) and the host's
    /// thread tries again later, sooner for a network failure than for a refused login.
    fn mark_lost(&self, e: &anyhow::Error) {
        let msg = format!("{e:#}");
        let auth = is_auth_error(&msg);
        self.link.fetch_add(1, Ordering::SeqCst);
        self.drop_connections();
        {
            let mut i = lock(&self.info);
            i.fails = i.fails.saturating_add(1);
            let wait = if auth { 60 } else { (5 * u64::from(i.fails)).min(60) };
            i.next_try = Instant::now() + Duration::from_secs(wait);
        }
        self.wake.notify_all();
        let code = if e.chain().count() == 1 { axelstra_core::msg::find(e).cloned() } else { None };
        self.set_state_coded(if auth { St::Auth } else { St::Lost }, Some(msg), code);
    }

    /// Drops every connection and kills the children (the event reader holds a clone that would keep its child alive).
    fn drop_connections(&self) {
        for s in [&self.control, &self.bg, &self.input, &self.watch] {
            if let Some(c) = lock(s).take() {
                c.kill_child();
            }
        }
    }

    fn open(&self) -> Result<Client> {
        let spec = self.def.remote_spec()?;
        Client::connect_remote(&spec, CONNECT_TIMEOUT)
    }

    /// Opens (or re-opens) the control connection, the event connection, and marks the host ok.
    pub fn connect_now(self: &Arc<Self>) -> Result<()> {
        let _gate = lock(&self.gate);
        if self.state() == St::Ok && lock(&self.control).is_some() {
            return Ok(());
        }
        self.set_state(St::Connecting, None);
        let result = (|| -> Result<()> {
            let c = self.open()?;
            let (proto, build, instance) = (c.proto, c.build.clone(), c.instance);
            *lock(&self.control) = Some(c);
            self.start_watch()?;
            {
                let mut i = lock(&self.info);
                i.proto = Some(proto);
                i.build = Some(build);
                i.fails = 0;
            }
            let prev = self.instance.swap(instance, Ordering::SeqCst);
            self.set_state(St::Ok, None);
            // a new daemon run on that host (first contact included): the page lists and restores its sessions again
            if prev != instance {
                emit("daemon-restarted", json!({"host": self.def.id}));
            }
            Ok(())
        })();
        if let Err(e) = &result {
            self.mark_lost(e);
        }
        result
    }

    /// Event connection: its end is how a dead link is noticed (ssh pings it every 15 s), so it is always open.
    fn start_watch(self: &Arc<Self>) -> Result<()> {
        let w = self.open().context("event connection")?;
        let mut rd = w.try_clone()?;
        *lock(&self.watch) = Some(w);
        let me = self.clone();
        let link = self.link.load(Ordering::SeqCst);
        std::thread::Builder::new().name(format!("host-events-{}", self.def.id)).spawn(move || {
            while let Ok(f) = rd.read() {
                if let Frame::Event(mut e) = f {
                    let name = e["event"].as_str().unwrap_or_default().to_string();
                    tag_result(&name, &me.def.id, me.def.slot, &mut e);
                    if let Some(ch) = lock(&EVENTS).as_ref() {
                        let _ = ch.send(InvokeResponseBody::Json(e.to_string()));
                    }
                }
            }
            if me.link.load(Ordering::SeqCst) == link && me.state() != St::Idle {
                me.mark_lost(&axelstra_core::coded!("host.link_ended", "the connection to {name} ended", name = me.def.name));
            }
        })?;
        Ok(())
    }

    /// A fresh connection for one terminal stream.
    pub fn connect_stream(self: &Arc<Self>) -> Result<Client> {
        self.require_up()?;
        self.open().inspect_err(|e| {
            if is_auth_error(&format!("{e:#}")) {
                self.mark_lost(e)
            }
        })
    }

    fn require_up(&self) -> Result<()> {
        let i = lock(&self.info);
        match i.state {
            St::Ok | St::Idle | St::Connecting => Ok(()),
            St::Lost | St::Auth => Err(axelstra_core::coded!(
                "host.not_connected",
                "no connection to {name} ({state}): {detail}",
                name = self.def.name,
                state = i.state.name(),
                detail = i.error.as_deref().unwrap_or("no detail").lines().next().unwrap_or("")
            )),
        }
    }

    fn ensure(self: &Arc<Self>, which: Which) -> Result<()> {
        self.require_up()?;
        if self.state() != St::Ok {
            self.connect_now()?;
        }
        if lock(self.slot(which)).is_some() {
            return Ok(());
        }
        let _gate = lock(&self.gate);
        if lock(self.slot(which)).is_some() {
            return Ok(());
        }
        let c = self.open().inspect_err(|e| self.mark_lost(e))?;
        *lock(self.slot(which)) = Some(c);
        Ok(())
    }
}

fn is_auth_error(msg: &str) -> bool {
    let m = msg.to_ascii_lowercase();
    [
        "permission denied",
        "host key verification failed",
        "authentication failed",
        "no more authentication methods",
        "too many authentication",
    ]
    .iter()
    .any(|p| m.contains(p))
}

/// `with_client` for a remote host: the same reconnect-once rules, a dead link is reported instead of retried at length.
pub fn with_remote<T>(h: &Arc<Remote>, which: Which, retry: bool, f: impl Fn(&mut Client) -> Result<T>) -> Result<T> {
    let slot = h.slot(which);
    let mut g = lock(slot);
    if g.is_none() {
        drop(g);
        h.ensure(which)?;
        g = lock(slot);
    }
    let Some(c) = g.as_mut() else {
        return Err(axelstra_core::coded!("host.no_link", "no connection to {name}", name = h.def.name));
    };
    match f(c) {
        Ok(v) => Ok(v),
        Err(e) if e.downcast_ref::<axelstra_daemon::client::Remote>().is_some() => Err(e),
        Err(e) => {
            let never_sent = e.downcast_ref::<axelstra_daemon::client::NotSent>().is_some();
            *g = None;
            drop(g);
            h.ensure(which)?;
            if retry || never_sent {
                match lock(slot).as_mut() {
                    Some(c) => f(c),
                    None => Err(e),
                }
            } else {
                Err(e.context(axelstra_core::coded!(
                    "host.link_dropped",
                    "the connection to {name} dropped during the request; check the state before repeating it",
                    name = h.def.name
                )))
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// calls

/// `call` on one host with the page's params already stripped; the answer comes back tagged.
fn call_host(h: &Arc<Remote>, which: Which, method: &str, params: Value, retry: bool) -> Result<Value> {
    let mut v = with_remote(h, which, retry, |c| c.call(method, params.clone()))?;
    if method == "project.add" {
        tag_project(&h.def.id, h.def.slot, &mut v);
    } else {
        tag_result(method, &h.def.id, h.def.slot, &mut v);
    }
    Ok(v)
}

/// A call made for a list: never waits longer than a few seconds, so one slow host cannot freeze the sidebar.
fn list_host(h: &Arc<Remote>, method: &str) -> Result<Value> {
    let mut v = with_remote(h, Which::Bg, true, |c| {
        c.timeout = Some(Duration::from_secs(8));
        let r = c.call(method, json!({}));
        c.timeout = None;
        r
    })?;
    if method == "project.list" {
        for p in v.as_array_mut().into_iter().flatten() {
            tag_project(&h.def.id, h.def.slot, p);
        }
    } else {
        tag_result(method, &h.def.id, h.def.slot, &mut v);
    }
    Ok(v)
}

/// What the page's `call` / `callBg` do once the allow-list passed: pick the host, run, tag.
/// `local` runs the call on this machine's daemon with the page's own params.
pub fn route(method: &str, params: Value, bg: bool, retry: bool, local: &dyn Fn(&str, Value) -> Result<Value>) -> Result<Value> {
    let which = if bg { Which::Bg } else { Which::Control };
    match plan(method, &params, &slot_host, &known).map_err(Coded::into_error)? {
        Plan::Local => local(method, params),
        Plan::Remote(id, p) => {
            let h = get(&id).ok_or_else(|| axelstra_core::coded!("host.unknown", "unknown host: {host}", host = id))?;
            call_host(&h, which, method, p, retry)
        }
        Plan::Split(calls) => {
            let mut answers = vec![];
            for (host, p) in calls {
                if host == LOCAL {
                    answers.push(local(method, p)?);
                } else if let Some(h) = get(&host) {
                    // a host that is down only loses its own rows
                    if let Ok(v) = call_host(&h, which, method, p, true) {
                        answers.push(v);
                    }
                }
            }
            Ok(join_answers(answers))
        }
        Plan::Fan => {
            let mut v = local(method, params)?;
            let up: Vec<Arc<Remote>> = all().into_iter().filter(|h| h.state() == St::Ok).collect();
            let extra: Vec<Value> = std::thread::scope(|s| {
                let jobs: Vec<_> = up.iter().map(|h| s.spawn(move || list_host(h, method))).collect();
                jobs.into_iter().filter_map(|j| j.join().ok()).filter_map(Result::ok).collect()
            });
            if let Some(a) = v.as_array_mut() {
                for e in extra {
                    if let Value::Array(items) = e {
                        a.extend(items);
                    }
                }
            }
            Ok(v)
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// event stream, input

pub fn set_events(ch: Channel<InvokeResponseBody>) {
    *lock(&EVENTS) = Some(ch);
}

/// Points the event stream at `worktree` (a tagged path for a remote one, a plain one for this machine, None to stop).
/// Returns the plain path the local daemon should watch, if any.
pub fn watch_target(worktree: Option<&str>) -> Result<Option<String>> {
    let new_host = worktree.and_then(untag).map(|(h, _)| h.to_string());
    let prev = std::mem::replace(&mut *lock(&WATCHING), new_host.clone());
    if let Some(p) = prev.filter(|p| Some(p) != new_host.as_ref()) {
        if let Some(h) = get(&p) {
            if let Some(c) = lock(&h.watch).as_mut() {
                let _ = c.send("watch.unsubscribe", json!({}));
            }
        }
    }
    match (worktree, new_host) {
        (Some(w), Some(host)) => {
            let h = get(&host).ok_or_else(|| axelstra_core::coded!("host.unknown", "unknown host: {host}", host = host))?;
            let rest = untag(w).map(|(_, r)| r).unwrap_or_default().to_string();
            h.require_up()?;
            let mut g = lock(&h.watch);
            let c = g.as_mut().ok_or_else(|| axelstra_core::coded!("host.no_link", "no connection to {name}", name = h.def.name))?;
            c.send("watch.subscribe", json!({"worktree": rest}))?;
            Ok(None)
        }
        (Some(w), None) => Ok(Some(w.to_string())),
        (None, _) => Ok(None),
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// registry management

fn persisted() -> Vec<Def> {
    crate::setting("hosts").as_array().map(|a| a.iter().filter_map(Def::from_json).collect()).unwrap_or_default()
}

/// Loads the registry saved in settings.json and starts one keeper thread per host.
pub fn load() {
    let mut hosts = lock(&HOSTS);
    for d in persisted() {
        if hosts.iter().any(|h| h.def.id == d.id) {
            continue;
        }
        let h = Remote::new(d);
        keep(h.clone());
        hosts.push(h);
    }
}

/// Connects a host at startup and again whenever it is lost, with a growing wait (a refused login waits longer).
fn keep(h: Arc<Remote>) {
    let _ = std::thread::Builder::new().name(format!("host-keeper-{}", h.def.id)).spawn(move || {
        // consecutive rounds where a connect left the host due again without changing anything (it should not happen:
        // `connect_now` either reaches Ok or `mark_lost` pushes `next_try` out); drives the backoff below
        let mut stuck = 0u32;
        loop {
            if get(&h.def.id).is_none_or(|x| !Arc::ptr_eq(&x, &h)) {
                return;
            }
            let i = lock(&h.info);
            let retry = matches!(i.state, St::Idle | St::Lost | St::Auth);
            if retry && Instant::now() >= i.next_try {
                drop(i);
                let _ = h.connect_now();
                // a connect that failed has pushed `next_try` out; one that worked changed the state: look again
                let i = lock(&h.info);
                if !matches!(i.state, St::Idle | St::Lost | St::Auth) || Instant::now() < i.next_try {
                    stuck = 0;
                    continue;
                }
                // still due after a connect (no progress): wait on the condvar with a backoff that grows with each such
                // round (1 s up to 30 s) instead of spinning; a state change or the host being removed wakes it at once
                let wait = Duration::from_secs(1 << stuck.min(5)).min(Duration::from_secs(30));
                stuck = stuck.saturating_add(1);
                drop(h.wake.wait_timeout(i, wait));
                continue;
            }
            stuck = 0;
            // asleep until the next try is due, or (connected, connecting) until the state changes or the host is removed
            if retry {
                let left = i.next_try.saturating_duration_since(Instant::now());
                drop(h.wake.wait_timeout(i, left));
            } else {
                drop(h.wake.wait(i));
            }
        }
    });
}

pub fn list() -> Vec<Value> {
    all().iter().map(|h| h.describe()).collect()
}

/// Saves `defs` through the local daemon, which validates them again (and refuses them to agents).
fn save(c: &mut Client, defs: &[Def]) -> Result<()> {
    c.call("settings.set", json!({"hosts": defs.iter().map(Def::to_json).collect::<Vec<_>>()}))?;
    Ok(())
}

pub fn add(c: &mut Client, name: &str, spec: &str, bin: Option<&str>) -> Result<Value> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 40 || name.chars().any(char::is_control) {
        return Err(axelstra_core::coded!("host.name_invalid_add", "invalid name: 1 to 40 characters"));
    }
    let parsed = RemoteSpec::parse(spec)?;
    let spec = spec.trim().to_string();
    let mut cur = persisted();
    if cur.len() >= MAX_HOSTS {
        return Err(axelstra_core::coded!("host.too_many", "at most {max} hosts", max = MAX_HOSTS));
    }
    if cur.iter().any(|d| d.spec == spec) {
        return Err(axelstra_core::coded!("host.spec_exists", "a host with the spec {spec} already exists", spec = spec));
    }
    let bin = bin.map(str::trim).filter(|b| !b.is_empty()).map(String::from);
    if let Some(b) = &bin {
        parsed.with_bin(b)?;
    }
    let id = slug(name, &|i| cur.iter().any(|d| d.id == i));
    let slot = (1..=MAX_SLOT)
        .find(|s| !cur.iter().any(|d| d.slot == *s))
        .ok_or_else(|| axelstra_core::coded!("host.no_free_slot", "no free slots"))?;
    let def = Def { id, name: name.to_string(), spec, slot, bin };
    cur.push(def.clone());
    save(c, &cur)?;
    let h = Remote::new(def);
    lock(&HOSTS).push(h.clone());
    keep(h.clone());
    emit("host-state", json!({"id": h.def.id, "state": "idle"}));
    Ok(h.describe())
}

pub fn remove(c: &mut Client, id: &str) -> Result<()> {
    let mut cur = persisted();
    let before = cur.len();
    cur.retain(|d| d.id != id);
    if cur.len() == before {
        return Err(axelstra_core::coded!("host.unknown", "unknown host: {host}", host = id));
    }
    save(c, &cur)?;
    let gone = {
        let mut hosts = lock(&HOSTS);
        let at = hosts.iter().position(|h| h.def.id == id);
        at.map(|i| hosts.remove(i))
    };
    if let Some(h) = gone {
        // dropping the connections ends their children; the sessions go on running on the host
        h.link.fetch_add(1, Ordering::SeqCst);
        h.set_state(St::Idle, None);
        h.drop_connections();
        // the keeper looks again and ends
        h.wake.notify_all();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hosts(slot: u32) -> Option<String> {
        match slot {
            1 => Some("lab".into()),
            2 => Some("wsl1".into()),
            _ => None,
        }
    }
    fn known_h(h: &str) -> bool {
        matches!(h, "lab" | "wsl1")
    }
    fn plan_of(m: &str, p: Value) -> Result<Plan, Coded> {
        plan(m, &p, &hosts, &known_h)
    }

    #[test]
    fn gid_roundtrip() {
        assert_eq!(split_gid(gid(0, 5)), (0, 5));
        assert_eq!(split_gid(gid(3, 77)), (3, 77));
        assert_eq!(gid(0, 9), 9);
        assert!(gid(MAX_SLOT, 0x00FF_FFFF) < (1 << 31));
    }

    #[test]
    fn untag_needs_a_valid_host() {
        assert_eq!(untag("@lab:/home/me"), Some(("lab", "/home/me")));
        assert_eq!(untag("@lab:C:\\x"), Some(("lab", "C:\\x")));
        assert_eq!(untag("@Lab:/x"), None);
        assert_eq!(untag("@:/x"), None);
        assert_eq!(untag("/home/me"), None);
        assert_eq!(untag("C:\\x"), None);
        assert_eq!(untag("@a b:/x"), None);
        assert_eq!(untag("@lab"), None);
    }

    #[test]
    fn local_calls_are_untouched() {
        assert_eq!(plan_of("fs.read", json!({"worktree": "C:\\p", "path": "a.rs"})), Ok(Plan::Local));
        assert_eq!(plan_of("session.kill", json!({"id": 5})), Ok(Plan::Local));
        assert_eq!(plan_of("settings.set", json!({"terse": "off"})), Ok(Plan::Local));
        // a message that happens to look like a prefix is not a place
        assert_eq!(plan_of("git.commit", json!({"worktree": "C:\\p", "message": "@lab:/x oops"})), Ok(Plan::Local));
    }

    #[test]
    fn remote_paths_and_ids_are_stripped() {
        let Ok(Plan::Remote(h, p)) = plan_of("fs.read", json!({"worktree": "@lab:/home/me/p", "path": "a.rs"})) else { panic!() };
        assert_eq!((h.as_str(), p["worktree"].as_str(), p["path"].as_str()), ("lab", Some("/home/me/p"), Some("a.rs")));
        let Ok(Plan::Remote(h, p)) = plan_of("session.kill", json!({"id": gid(2, 41)})) else { panic!() };
        assert_eq!((h.as_str(), p["id"].as_u64()), ("wsl1", Some(41)));
        let Ok(Plan::Remote(_, p)) = plan_of(
            "session.spawn",
            json!({"program": "bash", "cwd": "@lab:/x", "meta": {"worktree": "@lab:/x", "project": "@lab:abc", "parent": gid(1, 3)}}),
        ) else {
            panic!()
        };
        assert_eq!(p["cwd"], "/x");
        assert_eq!(p["meta"]["worktree"], "/x");
        assert_eq!(p["meta"]["project"], "abc");
        assert_eq!(p["meta"]["parent"], 3);
    }

    #[test]
    fn mixing_hosts_or_unknown_hosts_fail() {
        assert!(plan_of("worktree.rename", json!({"path": "@lab:/a", "name": "x", "parent": "@wsl1:/b"})).is_err());
        assert!(plan_of("session.kill", json!({"worktree": "@lab:/a", "id": gid(2, 1)})).is_err());
        assert!(plan_of("fs.read", json!({"worktree": "@nope:/a"})).unwrap_err().code == "host.unknown");
        assert!(plan_of("session.kill", json!({"id": gid(9, 1)})).is_err());
    }

    #[test]
    fn this_machines_only_methods_are_refused_for_remote_projects() {
        for m in ["docker.open_shell", "open.editor", "computer.enable", "automation.run"] {
            assert!(plan_of(m, json!({"worktree": "@lab:/x"})).unwrap_err().code == "host.remote_unsupported", "{m}");
        }
        assert_eq!(plan_of("docker.open_shell", json!({"worktree": "C:\\x"})), Ok(Plan::Local));
    }

    #[test]
    fn fan_out_and_split() {
        assert_eq!(plan_of("project.list", json!({})), Ok(Plan::Fan));
        assert_eq!(plan_of("session.list", json!({})), Ok(Plan::Fan));
        assert_eq!(plan_of("worktree.status", json!({"paths": ["C:\\a", "D:\\b"]})), Ok(Plan::Local));
        let Ok(Plan::Split(calls)) = plan_of("worktree.status", json!({"paths": ["C:\\a", "@lab:/b", "@lab:/c"]})) else { panic!() };
        assert_eq!(calls.len(), 2);
        let lab = calls.iter().find(|(h, _)| h == "lab").unwrap();
        assert_eq!(lab.1["paths"], json!(["/b", "/c"]));
        let local = calls.iter().find(|(h, _)| h == LOCAL).unwrap();
        assert_eq!(local.1["paths"], json!(["C:\\a"]));
        assert_eq!(join_answers(vec![json!({"status": [1]}), json!({"status": [2, 3]})]), json!({"status": [1, 2, 3]}));
    }

    #[test]
    fn host_methods_route_by_the_host_param() {
        assert_eq!(plan_of("host.info", json!({"host": "local"})), Ok(Plan::Local));
        let Ok(Plan::Remote(h, p)) = plan_of("host.browse", json!({"host": "lab", "path": "/home"})) else { panic!() };
        assert_eq!((h.as_str(), p.get("host").is_none(), p["path"].as_str()), ("lab", true, Some("/home")));
        assert!(plan_of("host.browse", json!({"host": "nope"})).is_err());
    }

    #[test]
    fn results_are_tagged() {
        let mut p = json!({"id": "abc", "name": "x", "path": "/home/me/p", "worktrees": [{"path": "/home/me/p", "parent": null}, {"path": "/home/me/p/.axelstra-wt/a", "parent": "/home/me/p"}]});
        tag_project("lab", 1, &mut p);
        assert_eq!(p["id"], "@lab:abc");
        assert_eq!(p["host"], "lab");
        assert_eq!(p["path"], "@lab:/home/me/p");
        assert_eq!(p["worktrees"][1]["path"], "@lab:/home/me/p/.axelstra-wt/a");
        assert_eq!(p["worktrees"][1]["parent"], "@lab:/home/me/p");
        assert!(p["worktrees"][0]["parent"].is_null());

        let mut s =
            json!([{"id": 4, "running": true, "label": "main", "meta": {"project": "abc", "worktree": "/w", "parent": 2, "key": "k"}}]);
        tag_result("session.list", "lab", 1, &mut s);
        assert_eq!(s[0]["id"], gid(1, 4));
        assert_eq!(s[0]["meta"]["parent"], gid(1, 2));
        assert_eq!(s[0]["meta"]["worktree"], "@lab:/w");
        assert_eq!(s[0]["meta"]["project"], "@lab:abc");

        // relative paths and non-session ids stay
        let mut f = json!({"entries": [{"name": "a", "dir": false}], "path": "src/a.rs", "id": 7});
        tag_result("fs.list", "lab", 1, &mut f);
        assert_eq!(f["path"], "src/a.rs");
        assert_eq!(f["id"], 7);

        let mut ev = json!({"event": "session.exit", "id": 9, "code": 0});
        tag_result("session.exit", "lab", 1, &mut ev);
        assert_eq!(ev["id"], gid(1, 9));
        let mut ev = json!({"event": "session.exited", "id": 9, "code": 3});
        tag_result("session.exited", "lab", 1, &mut ev);
        assert_eq!((&ev["id"], &ev["code"]), (&json!(gid(1, 9)), &json!(3)));
        let mut ev = json!({"event": "fs.changed", "worktree": "/home/me/p"});
        tag_result("fs.changed", "lab", 1, &mut ev);
        assert_eq!(ev["worktree"], "@lab:/home/me/p");
    }

    #[test]
    fn tagging_then_stripping_is_the_identity() {
        let mut v = json!({"worktree": "/home/me/p", "path": "/home/me/p/a", "meta": {"parent": 3}});
        let orig = v.clone();
        tag_result("session.spawn", "lab", 1, &mut v);
        let Ok(Plan::Remote(_, back)) = plan_of("session.spawn", v) else { panic!() };
        assert_eq!(back, orig);
    }

    #[test]
    fn slugs() {
        let none = |_: &str| false;
        assert_eq!(slug("Meu Servidor", &none), "meu-servidor");
        assert_eq!(slug("  --Lab_01!  ", &none), "lab-01");
        assert_eq!(slug("123", &none), "h123");
        assert_eq!(slug("local", &none), "local-2");
        assert_eq!(slug("lab", &|i| i == "lab" || i == "lab-2"), "lab-3");
        assert!(slug(&"x".repeat(60), &none).len() <= 24);
        assert!(valid_id(&slug("ÁÉÍ", &none)));
    }

    #[test]
    fn auth_errors_are_told_apart() {
        assert!(is_auth_error("user@h: Permission denied (publickey)."));
        assert!(is_auth_error("Host key verification failed."));
        assert!(!is_auth_error("ssh: connect to host h port 22: Connection refused"));
    }

    /// Against a real remote daemon: `AXELSTRA_TEST_HOST_SPEC=ssh:... AXELSTRA_TEST_HOST_PROJECT=/remote/git/repo
    /// cargo test -p axelstra-app real_remote -- --ignored --nocapture` (isolated data dir, see docs/TESTING.md).
    #[test]
    #[ignore]
    fn real_remote() {
        let (Ok(spec), Ok(project)) = (std::env::var("AXELSTRA_TEST_HOST_SPEC"), std::env::var("AXELSTRA_TEST_HOST_PROJECT")) else {
            panic!("set AXELSTRA_TEST_HOST_SPEC and AXELSTRA_TEST_HOST_PROJECT")
        };
        let h = Remote::new(Def { id: "rt".into(), name: "rt".into(), spec, slot: 5, bin: std::env::var("AXELSTRA_TEST_HOST_BIN").ok() });
        lock(&HOSTS).push(h.clone());
        h.connect_now().expect("connect");
        assert_eq!(h.state(), St::Ok);
        println!("hello: {}", h.describe());
        let none = |_: &str, _: Value| -> Result<Value> { anyhow::bail!("no local daemon in this test") };
        let go = |m: &str, p: Value| route(m, p, false, false, &none);

        let mut added = with_remote(&h, Which::Control, false, |c| c.call("project.add", json!({"path": project}))).unwrap();
        tag_project("rt", 5, &mut added);
        println!("project.add: {added}");
        let pid = added["id"].as_str().unwrap().to_string();
        let ppath = added["path"].as_str().unwrap().to_string();
        assert!(pid.starts_with("@rt:") && ppath.starts_with("@rt:/"));

        let list = list_host(&h, "project.list").unwrap();
        assert!(list.as_array().unwrap().iter().any(|p| p["id"] == pid && p["host"] == "rt"), "{list}");
        let fan = route("project.list", json!({}), false, true, &|_, _| Ok(json!([]))).unwrap();
        assert!(fan.as_array().unwrap().iter().any(|p| p["id"] == pid), "{fan}");

        let ls = go("fs.list", json!({"worktree": ppath, "path": ""})).unwrap();
        println!("fs.list: {ls}");
        assert!(ls["entries"].as_array().unwrap().iter().any(|e| e["name"] == "README.md"));
        let rd = go("fs.read", json!({"worktree": ppath, "path": "README.md"})).unwrap();
        assert!(rd["text"].as_str().unwrap().contains("hello"));
        let w = go(
            "fs.write",
            json!({"worktree": ppath, "path": "viaapp.txt", "text": "x
", "eol": "lf", "bom": false}),
        )
        .unwrap();
        println!("fs.write: {w}");
        let ch = go("git.changes", json!({"worktree": ppath})).unwrap();
        println!("git.changes: {ch}");
        assert!(ch["files"].as_array().unwrap().iter().any(|f| f["path"] == "viaapp.txt"), "{ch}");
        let c = go("git.commit", json!({"worktree": ppath, "message": "via app"})).unwrap();
        assert!(c["sha"].as_str().is_some());
        let log = go("git.log", json!({"worktree": ppath})).unwrap();
        assert!(log["commits"].as_array().unwrap().len() >= 2, "{log}");

        // a worktree of the remote project
        let wt = go("worktree.create", json!({"project": pid, "name": "wtest"})).unwrap();
        println!("worktree.create: {wt}");
        let wpath = wt["path"].as_str().unwrap().to_string();
        assert!(wpath.starts_with("@rt:/"));

        // a terminal in it
        let sp = go("session.spawn", json!({"program": "sh", "args": [], "label": "t", "cwd": wpath, "cols": 80, "rows": 24, "meta": {"worktree": wpath, "project": pid, "kind": "shell"}})).unwrap();
        let sid = sp["id"].as_u64().unwrap() as u32;
        assert_eq!(split_gid(sid).0, 5);
        let sess = route("session.list", json!({}), false, true, &|_, _| Ok(json!([]))).unwrap();
        assert!(sess.as_array().unwrap().iter().any(|x| x["id"] == sid && x["meta"]["worktree"] == wpath), "{sess}");
        let mut st = h.connect_stream().unwrap();
        let (_, raw) = split_gid(sid);
        st.call("session.subscribe", json!({"id": raw})).unwrap();
        with_remote(&h, Which::Input, true, |c| {
            c.send_input(
                raw,
                b"echo remote-$((6*7))
",
            )
        })
        .unwrap();
        let mut seen = String::new();
        let t = Instant::now();
        while t.elapsed() < Duration::from_secs(10) && !seen.contains("remote-42") {
            if let Ok(Frame::Output { data, .. }) = st.read() {
                seen.push_str(&String::from_utf8_lossy(&data));
            }
        }
        assert!(seen.contains("remote-42"), "terminal output: {seen}");
        go("session.kill", json!({"id": sid})).unwrap();
        go("worktree.remove", json!({"path": wpath, "project": pid})).ok();
        // refused for a remote project
        assert!(go("docker.open_shell", json!({"worktree": ppath})).is_err());
        // a lost link marks the host and the next call says so
        h.drop_connections();
        println!("done");
    }

    /// A link that dies and comes back: run it, then stop and start the remote (see the M7b report for the script).
    #[test]
    #[ignore]
    fn real_reconnect() {
        let spec = std::env::var("AXELSTRA_TEST_HOST_SPEC").expect("AXELSTRA_TEST_HOST_SPEC");
        let h = Remote::new(Def { id: "rc".into(), name: "rc".into(), spec, slot: 6, bin: None });
        lock(&HOSTS).push(h.clone());
        keep(h.clone());
        let mut seen: Vec<&'static str> = vec![];
        let t = Instant::now();
        while t.elapsed() < Duration::from_secs(170) {
            let st = h.state().name();
            if seen.last() != Some(&st) {
                println!("{:>5.1}s {st} {:?}", t.elapsed().as_secs_f32(), h.describe()["error"]);
                seen.push(st);
            }
            if seen.iter().filter(|s| **s == "ok").count() >= 2 && seen.contains(&"lost") {
                return;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        panic!("states seen: {seen:?}");
    }
}
