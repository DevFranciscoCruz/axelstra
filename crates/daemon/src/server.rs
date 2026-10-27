use crate::proto::*;
use crate::store::{read_json, write_atomic, write_json, LockRecover};
use anyhow::Result;
use axelstra_core::pty::{Session, Spawn};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::RecvTimeoutError;
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::{Duration, Instant};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::mpsc;
mod containers;
mod files;
mod messaging;
mod misc;
mod orchestration;
mod origin;
mod removal;
mod session;
mod sources;
mod triggers;
mod vcs;
mod worktrees;

/// Identifies this daemon run: a client that sees it change knows the daemon restarted (its sessions are gone).
static INSTANCE: std::sync::LazyLock<u64> = std::sync::LazyLock::new(|| {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
});

/// Frames queued for one connection's writer task. Bounded: a client that stops reading slows its own senders
/// down (they block) instead of growing the daemon's memory.
type Tx = mpsc::Sender<Vec<u8>>;
const OUT_QUEUE: usize = 1024;
/// How long a terminal stream waits for output before it sends an empty frame (see `session.subscribe`).
const STREAM_HEARTBEAT: Duration = Duration::from_secs(5);

/// Lock order. Hold at most these in this order, never a later one while taking an earlier one:
///   auto_lock, sessions, labels, metas, specs, pending, then the leaf locks (runs, flows, plist, watchers, the pulses).
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
    /// per-session capability tokens (env AXELSTRA_AGENT_TOKEN): a connection that presents one has role=agent
    agent_tokens: Mutex<HashMap<String, u32>>,
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
    /// what agents' own hooks said they are doing (`session.hint`)
    hints: axelstra_core::waiting::Hints,
    /// sessions that said they only wait for something external (`session.await`)
    awaits: axelstra_core::waiting::Awaits,
    /// sessions the stall watchdog already told their parent about (until they work again)
    stall_notified: Mutex<std::collections::HashSet<u32>>,
    /// bumped when a request changed projects or worktrees; watchers hear it as `projects.changed`
    wt_gen: AtomicU64,
    /// what agents asked the daemon to do (`audit.list`)
    audit: crate::audit::Audit,
    /// `hello`s refused with a bad token since the last good one: each failure makes the next answer slower
    bad_hellos: AtomicU32,
    /// bumped by anything that may change what `session.list` shows or whether the daemon is idle (a request, a
    /// connection coming or going, a session starting, exiting or printing after a quiet spell); see `spawn_session_monitor`
    pulse: Pulse,
    /// connections that get `session.changed` and `projects.changed` (their `watch.subscribe`), with their stop flag
    watchers: Mutex<Vec<(Arc<AtomicBool>, Tx)>>,
    /// the automation list changed: the scheduler works out its next due time again
    auto_wake: tokio::sync::Notify,
    /// the budgets in the settings changed: the budget loop reads them again
    budget_wake: tokio::sync::Notify,
    /// something may have made a worktree removable (a session ended, a merge, a finished flow): see `spawn_janitor`
    janitor: Pulse,
    /// `daemon.shutdown` is under way: the janitor must not start removing a worktree it could not finish
    closing: AtomicBool,
    /// worktree (normalized) -> when a session last left it, for the janitor's "used a moment ago" rule
    activity: Mutex<HashMap<String, Instant>>,
    /// when this daemon started: what it does not remember about a worktree it cannot rule out
    started: Instant,
    /// what the cleanup closed so far: sessions, processes, bytes of RAM (the app reads the difference of a manual run)
    closed: Closed,
    /// the last sessions that ended on their own and left the registry (`retire`): a leaf lock. `session.status` still
    /// answers for them, so `axelstra wait`, flows and the orchestrator get their exit code after the row is gone
    exited: Mutex<std::collections::VecDeque<Exited>>,
    /// the janitor's time constants (tests shorten them)
    timing: JanitorTiming,
    /// this registry as the background threads see it (set once the daemon serves): a save that is too soon after
    /// the last one is finished by a thread that needs it. Unset in unit tests, where a save is always immediate
    me: std::sync::OnceLock<std::sync::Weak<Registry>>,
    /// when `sessions.json` was last saved, and whether a save is already waiting for its turn
    save_turn: Mutex<SaveTurn>,
    /// the number of the last snapshot written and when the backups were last rotated; held while writing, so writes
    /// that race on disk keep the newest
    save_written: Mutex<(u64, Option<Instant>)>,
    /// numbers the snapshots of the saved state (taken under the `specs`/`pending` locks)
    save_gen: AtomicU64,
}

#[derive(Default)]
struct SaveTurn {
    last: Option<Instant>,
    queued: bool,
}

/// Saves of `sessions.json` closer together than this are one save, written at the end of the spell.
const SAVE_DEBOUNCE: Duration = Duration::from_millis(250);
/// The rotating backups of `sessions.json` are shifted at most this often.
const SAVE_ROTATE_EVERY: Duration = Duration::from_secs(5 * 60);

/// How long the janitor waits for things that become true with time, not with an event.
#[derive(Clone, Copy)]
struct JanitorTiming {
    /// a hand-made worktree must have had no session for this long (`janitor::MANUAL_QUIET_SECS`)
    quiet: Duration,
    /// kicks closer together than this make one sweep
    gap: Duration,
    /// the slow tick: a PR merged on GitHub raises no local event
    slow: Duration,
}

impl Default for JanitorTiming {
    fn default() -> Self {
        JanitorTiming {
            quiet: Duration::from_secs(crate::janitor::MANUAL_QUIET_SECS),
            gap: Duration::from_secs(60),
            slow: Duration::from_secs(30 * 60),
        }
    }
}

/// How long the janitor sleeps when no event wakes it: until the earliest moment a worktree stops counting as touched
/// (the daemon being too new to know, or a session that left `elapsed` ago), never longer than the slow tick.
fn janitor_wake(t: &JanitorTiming, started: Duration, left: impl Iterator<Item = Duration>) -> Duration {
    std::iter::once(started)
        .chain(left)
        .filter_map(|elapsed| t.quiet.checked_sub(elapsed).filter(|d| !d.is_zero()))
        .min()
        .map_or(t.slow, |d| (d + Duration::from_millis(20)).min(t.slow))
}

/// How many ended sessions `Registry::exited` remembers.
const EXITED_KEPT: usize = 256;

/// What is remembered of a session that ended and left the registry.
struct Exited {
    id: u32,
    code: Option<i32>,
    /// it had reported with `axelstra done`
    finished: bool,
}

/// Totals of what the janitor and the done-closer shut down.
#[derive(Default)]
struct Closed {
    sessions: AtomicU64,
    procs: AtomicU64,
    bytes: AtomicU64,
}

impl Closed {
    fn read(&self) -> (u64, u64, u64) {
        (self.sessions.load(Ordering::SeqCst), self.procs.load(Ordering::SeqCst), self.bytes.load(Ordering::SeqCst))
    }
}

/// Wakes a background waiter when something happened, so it never has to poll.
#[derive(Default)]
struct Pulse {
    count: Mutex<u64>,
    cv: Condvar,
}

impl Pulse {
    fn bump(&self) {
        *self.count.lock_recover() += 1;
        self.cv.notify_all();
    }

    fn now(&self) -> u64 {
        *self.count.lock_recover()
    }

    /// Waits for a bump after `seen` (the count it returned last time), or at most `timeout`. Returns the count now.
    fn wait(&self, seen: u64, timeout: Option<Duration>) -> u64 {
        let g = self.count.lock_recover();
        let g = match timeout {
            None => self.cv.wait_while(g, |n| *n == seen).unwrap_or_else(PoisonError::into_inner),
            Some(t) => self.cv.wait_timeout_while(g, t, |n| *n == seen).unwrap_or_else(PoisonError::into_inner).0,
        };
        *g
    }
}

impl Registry {
    /// Sends `{"event": name}` to every watching connection. One that stopped reading misses it rather than block us.
    fn broadcast(&self, name: &str) {
        self.broadcast_event(json!({"event": name}));
    }

    /// Same, for an event that carries data (`session.exited`).
    fn broadcast_event(&self, ev: Value) {
        let frame = encode(EVENT, ev.to_string().as_bytes());
        self.watchers.lock_recover().retain(|(stop, tx)| {
            !stop.load(Ordering::SeqCst) && !matches!(tx.try_send(frame.clone()), Err(mpsc::error::TrySendError::Closed(_)))
        });
    }

    /// Tells the app there is something to take for its toasts and notification center (`janitor.events`,
    /// `triggers.events`, `budget.events`); it takes them with those calls, so it never asks on a timer.
    fn announce_events(&self) {
        if crate::janitor::has_events() {
            self.broadcast("janitor.events");
        }
        if crate::triggers::has_events() {
            self.broadcast("triggers.events");
        }
        if crate::budget::has_events() {
            self.broadcast("budget.events");
        }
    }

    /// The worktrees changed outside a request that already says so: drop the cached lists and tell the app.
    fn projects_changed(&self) {
        forget_project_list(self);
        self.broadcast("projects.changed");
    }

    /// Asks for sessions.json to be rewritten. The first save after a quiet spell is written at once; ones that follow
    /// within `SAVE_DEBOUNCE` become one save at the end of the spell (restoring N sessions saves twice, not N times).
    /// `persist_now` is the save itself, for the moments that must not wait (shutdown).
    fn persist(&self) {
        let Some(me) = self.me.get().and_then(std::sync::Weak::upgrade) else { return self.persist_now() };
        let wait = {
            let mut turn = self.save_turn.lock_recover();
            let since = turn.last.map(|l| l.elapsed());
            match since.filter(|s| *s < SAVE_DEBOUNCE) {
                None => {
                    turn.last = Some(Instant::now());
                    None
                }
                Some(_) if turn.queued => return,
                Some(s) => {
                    turn.queued = true;
                    Some(SAVE_DEBOUNCE - s)
                }
            }
        };
        match wait {
            None => self.persist_now(),
            Some(wait) => {
                std::thread::spawn(move || {
                    std::thread::sleep(wait);
                    {
                        let mut turn = me.save_turn.lock_recover();
                        turn.queued = false;
                        turn.last = Some(Instant::now());
                    }
                    me.persist_now();
                });
            }
        }
    }

    /// Rewrites sessions.json: still-pending sessions first, then the live ones. A record of a deleted worktree or of a
    /// session that reported with `axelstra done` is not kept: it is dropped here, with its `sid/` file, so the
    /// clean-up is continuous (every save) and not only the one at start. The vector is built from a snapshot taken under
    /// the registry locks; the folder checks and the disk write happen after they are released.
    fn persist_now(&self) {
        let (pending, specs, gen) = {
            let specs = self.specs.lock_recover();
            let pending = self.pending.lock_recover();
            let mut ids: Vec<_> = specs.keys().copied().collect();
            ids.sort();
            let live: Vec<(u32, Value)> = ids.into_iter().map(|i| (i, specs[&i].clone())).collect();
            (pending.clone(), live, self.save_gen.fetch_add(1, Ordering::SeqCst) + 1)
        };
        let dead: Vec<Value> = pending.iter().filter(|r| !rec_alive(r)).cloned().collect();
        if !dead.is_empty() {
            self.pending.lock_recover().retain(|r| !dead.contains(r));
            for r in &dead {
                self.drop_sid_of(r);
            }
        }
        let mut v: Vec<Value> = pending.into_iter().filter(|r| !dead.contains(r)).collect();
        // a session that reported with `axelstra done` is finished: restoring it would only start an agent with nothing to do
        v.extend(specs.into_iter().filter(|(i, s)| self.bus.finished(*i).is_none() && rec_alive(s)).map(|(_, s)| s));
        // the bus forgets a `done` with the daemon: the mark in the spec keeps a finished task from coming back
        v.retain(|r| r["meta"]["done"] != true);
        // a bare shell (no folder, no meta) is not kept; one conversation (same key) is saved once
        v.retain(|r| !rec_junk(r));
        let mut keys = std::collections::HashSet::new();
        v.retain(|r| r["meta"]["key"].as_str().is_none_or(|k| keys.insert(k.to_string())));
        if real_dir_under_test(&self.saved) {
            return;
        }
        // one writer at a time; a snapshot older than the one already on disk is not written over it
        let mut written = self.save_written.lock_recover();
        if gen <= written.0 {
            return;
        }
        let rotate = written.1.is_none_or(|at| at.elapsed() >= SAVE_ROTATE_EVERY);
        if crate::store::write_json_with(&self.saved, &v, rotate).is_ok() {
            *written = (gen, if rotate { Some(Instant::now()) } else { written.1 });
        }
    }

    /// Marks (or unmarks, when new work reaches it) the saved spec of `id` as finished, so a restart does not bring a
    /// finished task back, and saves.
    fn set_done(&self, id: u32, done: bool) {
        let changed = {
            let mut specs = self.specs.lock_recover();
            match specs.get_mut(&id).filter(|s| s["meta"].is_object()) {
                Some(s) if (s["meta"]["done"] == true) != done => {
                    if done {
                        s["meta"]["done"] = json!(true);
                    } else {
                        s["meta"].as_object_mut().map(|o| o.remove("done"));
                    }
                    true
                }
                _ => false,
            }
        };
        if changed {
            self.persist();
        }
    }

    /// `sid/` and `scrollback/` files no saved or live session refers to (old enough not to be one being written).
    fn clean_leftovers(&self, dry: bool) -> Vec<Value> {
        let mut keys: Vec<String> = vec![];
        let key_of = |r: &Value| r["meta"]["key"].as_str().map(String::from);
        keys.extend(self.pending.lock_recover().iter().filter_map(key_of));
        keys.extend(self.specs.lock_recover().values().filter_map(key_of));
        keys.extend(self.metas.lock_recover().values().filter_map(|m| m["key"].as_str().map(String::from)));
        let mut notes: Vec<String> = vec![];
        let note_of = |r: &Value| r["meta"]["note"].as_str().map(String::from);
        let mut refs = String::new();
        {
            let pending = self.pending.lock_recover();
            let specs = self.specs.lock_recover();
            notes.extend(pending.iter().filter_map(note_of));
            notes.extend(specs.values().filter_map(note_of));
            refs.extend(pending.iter().chain(specs.values()).map(|r| r.to_string()));
        }
        notes.extend(self.metas.lock_recover().values().filter_map(|m| m["note"].as_str().map(String::from)));
        let mut out = vec![];
        if let Some(dir) = self.saved.parent() {
            crate::cleanup::leftovers(dir, &keys, dry, &mut out);
            crate::cleanup::prompt_leftovers(dir, &refs, dry, &mut out);
            let wts = crate::cleanup::live_worktrees(&crate::projects::load(&self.store));
            crate::cleanup::worktree_leftovers(dir, wts.as_deref(), &notes, dry, &mut out);
        }
        out
    }

    /// Deletes the files a saved session left behind: the `sid/<key>.json` the hook wrote (the conversation id used to
    /// resume it) and its saved terminal history.
    fn drop_sid_of(&self, rec: &Value) {
        let Some(key) = rec["meta"]["key"].as_str().filter(|k| !k.is_empty() && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
        else {
            return;
        };
        let _ = std::fs::remove_file(self.saved.with_file_name("sid").join(format!("{key}.json")));
        let _ = std::fs::remove_file(self.saved.with_file_name("scrollback").join(format!("{key}.txt")));
    }

    fn new(store: std::path::PathBuf, pipe: String, token: String) -> Registry {
        let saved = store.with_file_name("sessions.json");
        // a corrupt sessions.json is moved to sessions.json.bad, so the first persist cannot wipe it
        let mut pending = read_json::<Vec<Value>>(&saved).ok().flatten().unwrap_or_default();
        prune_dead_pending(&saved, &mut pending);
        let audit = crate::audit::Audit::new(Some(saved.with_file_name("audit.log")));
        Registry {
            next: AtomicU32::new(0),
            sessions: Default::default(),
            labels: Default::default(),
            metas: Default::default(),
            store,
            pipe,
            token,
            agent_tokens: Default::default(),
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
            hints: Default::default(),
            awaits: Default::default(),
            stall_notified: Default::default(),
            wt_gen: AtomicU64::new(0),
            audit,
            bad_hellos: AtomicU32::new(0),
            pulse: Default::default(),
            watchers: Default::default(),
            auto_wake: Default::default(),
            budget_wake: Default::default(),
            janitor: Default::default(),
            closing: AtomicBool::new(false),
            activity: Default::default(),
            started: Instant::now(),
            closed: Default::default(),
            exited: Default::default(),
            timing: Default::default(),
            me: Default::default(),
            save_turn: Default::default(),
            save_written: Mutex::new((0, None)),
            save_gen: AtomicU64::new(0),
        }
    }

    /// Throttle for token guessing: the n-th refusal in a row waits n * 50 ms (at most 1 s) before it is answered.
    fn hello_failed(&self) {
        let n = self.bad_hellos.fetch_add(1, Ordering::SeqCst).saturating_add(1);
        std::thread::sleep(Duration::from_millis(u64::from(n.min(20)) * 50));
    }

    /// True when the agent in this session runs without permission prompts: whatever is typed and sent with Enter runs.
    fn runs_in_bypass(&self, id: u32) -> bool {
        let spec = self.specs.lock_recover().get(&id).map(|s| s.to_string().to_ascii_lowercase()).unwrap_or_default();
        ["bypasspermissions", "dangerously", "--yolo", "never-ask"].iter().any(|k| spec.contains(k))
    }

    /// Types text that came from another session into an agent. No Enter follows when that agent runs in bypass mode:
    /// a hostile message would otherwise execute unreviewed.
    fn deliver_untrusted(&self, id: u32, target: Arc<Session>, text: &str) {
        crate::msgbus::deliver(target, crate::msgbus::paste_with(text, !self.runs_in_bypass(id)));
    }

    /// True while a session waiting to be restored (from the previous daemon run) belongs to this worktree.
    fn worktree_pending(&self, path: &str) -> bool {
        let norm = |s: &str| s.replace('\\', "/").trim_end_matches('/').to_ascii_lowercase();
        let want = norm(path);
        self.pending.lock_recover().iter().any(|r| {
            r["meta"]["worktree"].as_str().is_some_and(|w| norm(w) == want) || r["cwd"].as_str().is_some_and(|c| within(&norm(c), &want))
        })
    }

    /// True while a live session works in this worktree (any terminal or agent with it as worktree or cwd).
    /// What the session said it waits for: an unanswered `ask`/`escalate`, or (unless it reported with `done`) its
    /// agent's own hook event. Never the screen text.
    fn wait_of(&self, id: u32, since_input_ms: u64, idle_ms: u64) -> Option<axelstra_core::waiting::Wait> {
        if let Some(text) = self.bus.pending_of(id) {
            return Some(axelstra_core::waiting::Wait { kind: axelstra_core::waiting::WaitKind::Question, text });
        }
        if self.bus.finished(id).is_some() {
            return None;
        }
        self.hints.waiting(id, since_input_ms, idle_ms)
    }

    /// What the session said it only waits for (CI, a deploy), while that still holds. Never after `done`.
    fn awaiting_of(&self, id: u32, since_input_ms: u64, idle_ms: u64) -> Option<axelstra_core::waiting::AwaitView> {
        if self.bus.finished(id).is_some() {
            return None;
        }
        self.awaits.active(id, since_input_ms, idle_ms)
    }

    /// Tells the session's parent (once per wait) that it waits for an answer, with the agent's own text.
    fn tell_parent_waiting(&self, id: u32) {
        let Some(wait) = self.hints.take_untold(id) else { return };
        let Some(pid) = self.metas.lock_recover().get(&id).and_then(|m| m["parent"].as_u64()).map(|p| p as u32) else { return };
        let Some(ps) = self.get(pid) else { return };
        let label = self.labels.lock_recover().get(&id).cloned().unwrap_or_else(|| crate::prompts::session_name(id));
        let text = crate::prompts::waiting_report(wait.kind == axelstra_core::waiting::WaitKind::Permission, &wait.text);
        let m = self.bus.send(Some(id), pid, "message", &text, None, None);
        let agent = self.metas.lock_recover().get(&pid).map(|m| m["kind"] == "agent").unwrap_or(false);
        if agent {
            self.deliver_untrusted(pid, ps, &crate::msgbus::notice(&m, &format!("{label} ({})", crate::prompts::session_name(id))));
        }
    }

    /// Sessions that are alive and have this worktree as worktree or cwd.
    fn live_in(&self, path: &str) -> Vec<(u32, Arc<Session>)> {
        let norm = |s: &str| s.replace('\\', "/").trim_end_matches('/').to_ascii_lowercase();
        let want = norm(path);
        // one lock at a time (the rest of the daemon takes sessions before metas)
        let alive: Vec<(u32, Arc<Session>)> =
            self.sessions.lock_recover().iter().filter(|(_, s)| s.try_wait().is_none()).map(|(id, s)| (*id, s.clone())).collect();
        let metas = self.metas.lock_recover();
        let in_meta = |id: &u32| metas.get(id).and_then(|m| m["worktree"].as_str()).map(|w| norm(w) == want).unwrap_or(false);
        let by_meta: Vec<u32> = alive.iter().map(|(id, _)| *id).filter(in_meta).collect();
        drop(metas);
        let specs = self.specs.lock_recover();
        let by_cwd = |id: &u32| specs.get(id).and_then(|sp| sp["cwd"].as_str()).map(|c| within(&norm(c), &want)).unwrap_or(false);
        alive.into_iter().filter(|(id, _)| by_meta.contains(id) || by_cwd(id)).collect()
    }

    /// A live session that has nothing left to do in its worktree: an agent that reported with `axelstra done` and went
    /// quiet (it only waits to be closed), or a terminal nobody typed in or saw output from for a long while and that
    /// runs nothing. Such a session does not keep its worktree from being cleaned; the janitor closes it first.
    fn releasable(&self, id: u32, s: &Session) -> bool {
        let quiet = |ms: u64| s.idle_ms() >= ms && s.since_input_ms() >= ms;
        if self.bus.finished(id).is_some() {
            return quiet(REPORTED_IDLE_MS);
        }
        let agent = self.metas.lock_recover().get(&id).is_some_and(|m| m["kind"] == "agent");
        if agent || !quiet(SHELL_IDLE_MS) {
            return false;
        }
        // a dev server or a build in the terminal is work, however quiet
        let kids = s.pid().map(|pid| axelstra_core::procs::with_descendants(&[pid]).len()).unwrap_or(1);
        kids <= 1
    }

    /// Notes that a session left this worktree just now.
    fn note_activity(&self, path: &str) {
        self.activity.lock_recover().insert(crate::projects::norm_path(path), Instant::now());
    }

    /// For a worktree the user made by hand: a session of any kind is open there, or one was a moment ago (or this
    /// daemon is too new to know).
    fn worktree_touched(&self, path: &str) -> bool {
        let quiet = self.timing.quiet;
        if self.started.elapsed() < quiet || !self.live_in(path).is_empty() {
            return true;
        }
        self.activity.lock_recover().get(&crate::projects::norm_path(path)).is_some_and(|at| at.elapsed() < quiet)
    }

    /// When the janitor must look again without any event (see `janitor_wake`). Forgets activity notes that no longer
    /// matter.
    fn next_janitor_wake(&self) -> Duration {
        let mut activity = self.activity.lock_recover();
        activity.retain(|_, at| at.elapsed() < self.timing.quiet);
        janitor_wake(&self.timing, self.started.elapsed(), activity.values().map(Instant::elapsed))
    }

    /// Closes every session of this worktree (agents, shells, whatever they started: the whole process tree dies) and
    /// waits until they are gone, so the folder is no longer anyone's cwd. Returns their labels.
    fn close_worktree(&self, path: &str) -> Vec<String> {
        let doomed = self.live_in(path);
        if doomed.is_empty() {
            return vec![];
        }
        let sessions: Vec<Arc<Session>> = doomed.iter().map(|(_, s)| s.clone()).collect();
        self.tear_down(&sessions);
        let mut labels = vec![];
        for (id, _) in &doomed {
            labels.push(self.labels.lock_recover().get(id).cloned().unwrap_or_else(|| format!("sessão {id}")));
            self.forget_session(*id);
        }
        self.note_activity(path);
        self.persist();
        self.pulse.bump();
        labels
    }

    /// Kills the sessions' process trees and waits until they are gone, then kills whatever of the trees outlived the
    /// kill (a process that broke out of its job), and counts sessions, processes and RAM released.
    fn tear_down(&self, sessions: &[Arc<Session>]) {
        let snap = axelstra_core::procs::snapshot();
        let roots: Vec<u32> = sessions.iter().filter_map(|s| s.pid()).collect();
        let tree = snap.descendants(&roots);
        let bytes: u64 = sessions.iter().map(|s| s.memory_now_in(&snap)).sum();
        kill_and_wait(sessions, Duration::from_secs(5));
        let left = axelstra_core::procs::alive_of(&tree);
        let stray: std::collections::HashSet<u32> = left.into_iter().filter(|p| !roots.contains(p)).collect();
        axelstra_core::procs::kill_pids(&stray);
        self.closed.sessions.fetch_add(sessions.len() as u64, Ordering::SeqCst);
        self.closed.procs.fetch_add(tree.len() as u64, Ordering::SeqCst);
        self.closed.bytes.fetch_add(bytes, Ordering::SeqCst);
    }

    /// Takes a finished session out of the registry (its tab, its restore record, its bus state). False when it was
    /// not there (something else removed it first).
    fn forget_session(&self, id: u32) -> bool {
        let was = self.sessions.lock_recover().remove(&id).is_some();
        self.labels.lock_recover().remove(&id);
        self.metas.lock_recover().remove(&id);
        let gone = self.specs.lock_recover().remove(&id);
        if let Some(spec) = gone {
            self.drop_sid_of(&spec);
        }
        self.bus.forget(id);
        was
    }

    /// A session whose program ended on its own leaves the registry, so `session.list`, `axelstra ls` and
    /// `sessions.json` show only what runs. Tells the watchers `session.exited {id, code}` (the app keeps a failed
    /// terminal's tab until the user closes it, and shows the code) and `dispatch.changed`, and wakes the session
    /// monitor and the janitor. Its exit code stays answerable through `session.status` (`exited`). Only the one that
    /// really removed it announces, so a kill racing the exit does not announce twice. Whatever it reported with
    /// `axelstra done` is already in its parent's inbox and its task file (written when `done` was called).
    fn retire(&self, id: u32, code: Option<i32>) -> bool {
        let finished = self.bus.finished(id).is_some();
        // a closed tab keeps no history: the periodic save would otherwise leave its file behind
        let key = self.metas.lock_recover().get(&id).and_then(|m| m["key"].as_str().map(String::from));
        if !self.forget_session(id) {
            return false;
        }
        if let Some(f) = key.and_then(|k| history_file(self, &k)) {
            let _ = std::fs::remove_file(f);
        }
        {
            let mut ring = self.exited.lock_recover();
            ring.push_back(Exited { id, code, finished });
            while ring.len() > EXITED_KEPT {
                ring.pop_front();
            }
        }
        self.persist();
        self.broadcast_event(json!({"event": "session.exited", "id": id, "code": code}));
        self.broadcast("dispatch.changed");
        self.pulse.bump();
        self.janitor.bump();
        true
    }

    /// `(code, finished)` of a session that ended and left the registry, if it is still remembered.
    fn exited_of(&self, id: u32) -> Option<(Option<i32>, bool)> {
        self.exited.lock_recover().iter().rev().find(|e| e.id == id).map(|e| (e.code, e.finished))
    }

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
        in_flow || self.orch_active(path) || self.live_in(path).iter().any(|(id, s)| !self.releasable(*id, s))
    }

    /// True while an orchestrator run of this project is going and `path` is one of its worktrees (`<name>-integ`,
    /// `<name>-<card>` in `.axelstra-wt`): the run owns them until it ends, whatever the janitor thinks of their branches.
    fn orch_active(&self, path: &str) -> bool {
        let p = std::path::Path::new(path);
        let (Some(dir), Some(leaf)) = (p.parent().and_then(|d| d.file_name()), p.file_name().map(|l| l.to_string_lossy())) else {
            return false;
        };
        dir == ".axelstra-wt"
            && self.runs.lock_recover().values().any(|r| {
                r["state"] == "running"
                    && r["name"].as_str().is_some_and(|n| leaf.strip_prefix(n).is_some_and(|rest| rest.starts_with('-')))
            })
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

    /// The running session that carries conversation key `key` (`meta.key`), if any.
    fn live_by_key(&self, key: &str) -> Option<u32> {
        if key.is_empty() {
            return None;
        }
        let sessions = self.sessions.lock_recover();
        let metas = self.metas.lock_recover();
        metas
            .iter()
            .find(|(id, m)| m["key"].as_str() == Some(key) && sessions.get(id).is_some_and(|s| s.try_wait().is_none()))
            .map(|(id, _)| *id)
    }

    fn get(&self, id: u32) -> Option<Arc<Session>> {
        self.sessions.lock_recover().get(&id).cloned()
    }

    /// Starts a session and records its label and UI metadata.
    fn spawn(self: &Arc<Self>, s: &Spawn, label: Option<&str>, meta: Option<Value>) -> Result<u32> {
        // the session learns its own id, so a CLI run inside it can name it as the parent of a worker
        let id = self.next.fetch_add(1, Ordering::SeqCst) + 1;
        let mut s = s.clone();
        s.env.push(("AXELSTRA_SESSION".into(), id.to_string()));
        // the session's own credential: it identifies the connection as an agent, which cannot do the app-only calls
        s.env.retain(|(k, _)| k != "AXELSTRA_AGENT_TOKEN");
        if let Some(t) = new_agent_token() {
            self.agent_tokens.lock_recover().insert(t.clone(), id);
            s.env.push(("AXELSTRA_AGENT_TOKEN".into(), t));
        }
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
        // working again after a quiet spell: the session monitor hears it without polling (weak: no cycle through the hook)
        let weak = Arc::downgrade(self);
        sess.on_wake(Arc::new(move || {
            if let Some(r) = weak.upgrade() {
                r.pulse.bump();
            }
        }));
        self.sessions.lock_recover().insert(id, sess.clone());
        if let Some(l) = label {
            self.labels.lock_recover().insert(id, l.to_string());
        }
        if let Some(m) = meta {
            self.metas.lock_recover().insert(id, m);
        }
        watch_exit(self, id, sess);
        self.pulse.bump();
        Ok(id)
    }
}

/// Waits (no polling, see `Session::wait_exit`) for the program of session `id` to end, then: whatever it left running
/// in its process tree goes too (a dev server or an agent's helpers would otherwise keep using memory with nothing to
/// show them), watchers hear it, and the janitor looks for a worktree this freed.
/// The tab itself stays (it shows the exit code) until the app removes it.
fn watch_exit(reg: &Arc<Registry>, id: u32, s: Arc<Session>) {
    let reg = Arc::downgrade(reg);
    std::thread::spawn(move || {
        s.wait_exit();
        s.kill();
        // the last output may still be on its way through the pipe: then the stream ends, which is what tells a
        // terminal's subscribers that the program is gone
        std::thread::sleep(Duration::from_millis(150));
        s.end_stream();
        let Some(reg) = reg.upgrade() else { return };
        if let Some(w) = reg.metas.lock_recover().get(&id).and_then(|m| m["worktree"].as_str().map(String::from)) {
            reg.note_activity(&w);
        }
        reg.hints.clear(id);
        reg.awaits.clear(id);
        // a session that ended (killed, `exit`, finished) is not brought back by a restart, and leaves the registry
        // (`retire`: list, `ls`, sessions.json, `session.exited`). A daemon shutdown keeps them on purpose (`closing`):
        // those are restored on the next start.
        if !reg.closing.load(Ordering::SeqCst) {
            if reg.bus.is_reaped(id) {
                // an ephemeral worker or lead: its reaper reports to the parent first and retires it right after
                let gone = reg.specs.lock_recover().remove(&id);
                if let Some(spec) = gone {
                    reg.drop_sid_of(&spec);
                    reg.persist();
                }
            } else {
                // killed or removed meanwhile: gone from the registry already, nothing to announce
                reg.retire(id, s.try_wait());
            }
        }
        reg.pulse.bump();
        reg.janitor.bump();
    });
}

/// A terminal size from a request: `as u16` would wrap 65537 to 1, and 0 is no size at all.
fn term_dim(v: &Value, default: u16) -> u16 {
    v.as_u64().map_or(default, |x| x.clamp(1, 1000) as u16)
}

/// What `hello` and `ping` say about this daemon, so the app can tell when it is older than it should be.
fn hello_info() -> Value {
    json!({"proto": PROTO, "instance": *INSTANCE, "build": BUILD_ID, "security": SECURITY_MIN,
        "version": axelstra_core::buildinfo::VERSION, "commit": axelstra_core::buildinfo::COMMIT, "built": axelstra_core::buildinfo::BUILT})
}

fn new_agent_token() -> Option<String> {
    let mut b = [0u8; 24];
    getrandom::getrandom(&mut b).ok()?;
    Some(b.iter().map(|x| format!("{x:02x}")).collect())
}

/// Calls that only the app (the user) may make; a connection with role=agent is refused them.
fn agent_forbidden(method: &str, p: &Value) -> bool {
    match method {
        "computer.enable" => p["on"].as_bool() == Some(true),
        // an agent must not raise its own limit or switch the auto-pause off; the events are the user's to take
        "budget.check" | "budget.events" => true,
        // they list the file system outside any worktree
        "host.info" | "host.browse" => true,
        // notifications, the stall watchdog, installs and the shell are what keeps the user informed and safe
        "settings.set" => ["budgets", "hosts", "notify_mode", "notify_sound", "stall_minutes", "auto_install", "shell"]
            .iter()
            .any(|k| p.get(*k).is_some()),
        // the saved sessions are the user's to drop, and an agent must not stop the daemon or switch the clean-up off
        "daemon.shutdown" | "session.pending_clear" => true,
        "janitor.auto" => p.get("on").is_some(),
        "project.add"
        | "project.remove"
        | "project.remove_preview"
        | "project.setup"
        | "hooks.install"
        | "automation.save"
        | "orch.start"
        | "notices.take"
        | "audit.list"
        | "backup.export"
        | "backup.restore"
        | "gh.send_comment"
        | "triggers.set"
        | "triggers.events"
        | "worktree.rebase"
        | "worktree.integrate"
        | "project.integrate"
        | "project.discover"
        | "project.add_discovered"
        | "project.clone"
        | "project.clone_status"
        | "project.clone_cancel"
        | "project.init"
        | "gh.repos"
        | "workspace.export"
        | "workspace.import_preview"
        | "workspace.import"
        // opening a browser is the user's act: the app host calls it, never an agent
        | "files.resolve_html"
        // describing, revealing, opening or RUNNING a path: the user confirmed it in the app, an agent must never
        | "files.resolve_path"
        | "files.path_action" => true,
        _ => false,
    }
}

/// True when the agent session `me` may drive `target`: it is `me` itself or a session `me` started, found by walking
/// the bus parent chain up from `target`. A lead keeps driving its workers; a worker cannot touch its siblings or parents.
fn agent_drives(reg: &Registry, me: u32, target: u32) -> bool {
    let mut cur = target;
    // the chain is short; the bound only protects against a cycle
    for _ in 0..64 {
        if cur == me {
            return true;
        }
        match reg.bus.parent_of(cur) {
            Some(parent) => cur = parent,
            None => return false,
        }
    }
    false
}

/// The refusal an agent gets when it reaches for a session that is not its own or one of its descendants.
fn not_own_tree(method: &str) -> anyhow::Error {
    axelstra_core::coded!(
        "rpc.agent_own_tree",
        "{method} is only allowed for the agent's own session and the sessions it started",
        method = method
    )
}

/// True when `path` is `root` or lies below it, comparing whole path components (`/a/wt-2` is not inside `/a/wt`).
/// Both are already normalized (forward slashes, lowercase).
/// False when a saved session points at folders that are all gone (a deleted worktree): it cannot be restored.
/// A record with no folder at all (a plain shell) counts as alive.
fn rec_alive(rec: &Value) -> bool {
    let dirs: Vec<&str> = [rec["meta"]["worktree"].as_str(), rec["cwd"].as_str()].into_iter().flatten().filter(|d| !d.is_empty()).collect();
    dirs.is_empty() || dirs.iter().any(|d| std::path::Path::new(d).exists())
}

/// One-off clean-up at start: sessions saved for worktrees that no longer exist are dropped, so a restart does not
/// try to bring back dozens of dead agents. The file as it was is copied to `backups/` first.
fn prune_dead_pending(saved: &std::path::Path, pending: &mut Vec<Value>) {
    if real_dir_under_test(saved) {
        return;
    }
    // dead worktrees, shells that carry nothing worth bringing back, and a second entry of one conversation (same key)
    let keep = |r: &Value, keys: &mut std::collections::HashSet<String>| {
        rec_alive(r) && !rec_junk(r) && r["meta"]["key"].as_str().is_none_or(|k| keys.insert(k.to_string()))
    };
    let mut keys = std::collections::HashSet::new();
    if pending.iter().all(|r| keep(r, &mut keys)) {
        return;
    }
    if let (Some(dir), Ok(ts)) = (saved.parent(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)) {
        let backups = dir.join("backups");
        if std::fs::create_dir_all(&backups).is_ok() {
            let _ = std::fs::copy(saved, backups.join(format!("sessions-before-prune-{}.json", ts.as_secs())));
        }
    }
    let mut keys = std::collections::HashSet::new();
    pending.retain(|r| keep(r, &mut keys));
    let _ = write_json(saved, &*pending);
}

/// A saved shell with no folder and nothing in its meta (no project, worktree, kind or key): nobody opened it on purpose
/// in a worktree, so restoring it only piles up bare `cmd`/`powershell` windows on every start. Terminals the user opens
/// and agents always carry a cwd or a meta.
fn rec_junk(rec: &Value) -> bool {
    let has = |v: &Value| v.as_str().is_some_and(|s| !s.is_empty());
    let meta = &rec["meta"];
    !has(&rec["cwd"]) && !["worktree", "project", "key", "kind"].iter().any(|k| has(&meta[*k]))
}

/// Under test, the real data dir's files are never written (the same rule `serve_with_store` applies to the daemon).
fn real_dir_under_test(saved: &std::path::Path) -> bool {
    saved.parent().is_some_and(|d| crate::proto::refuse_real_when_testing("", d).is_err())
}

fn within(path: &str, root: &str) -> bool {
    std::path::Path::new(path).starts_with(root)
}

pub(crate) const SHELLS: [&str; 6] = ["auto", "pwsh", "powershell", "bash", "wsl", "cmd"];
pub(crate) const NOTIFY_MODES: [&str; 3] = ["all", "waiting", "off"];

/// `shell`, `notify_sound` and `notify_mode` as saved, each falling back to its default (auto, true, all) when missing or invalid.
/// The app itself reads the same keys from settings.json.
fn ui_settings(saved: &Value) -> (&str, bool, &str) {
    let shell = saved["shell"].as_str().filter(|s| SHELLS.contains(s)).unwrap_or("auto");
    let mode = saved["notify_mode"].as_str().filter(|m| NOTIFY_MODES.contains(m)).unwrap_or("all");
    (shell, saved["notify_sound"] != json!(false), mode)
}

/// The error of a request without a required parameter.
pub(crate) fn missing(name: &str) -> anyhow::Error {
    axelstra_core::coded!("rpc.missing_param", "missing {name}", name = name)
}

/// Copies the keys of a `settings.set` patch that are present into `saved`, rejecting invalid values.
pub(crate) fn apply_ui_settings(saved: &mut Value, p: &Value) -> Result<()> {
    if let Some(s) = p["shell"].as_str() {
        if !SHELLS.contains(&s) {
            return Err(axelstra_core::coded!("settings.bad_shell", "shell must be auto, pwsh, powershell, bash, wsl or cmd"));
        }
        saved["shell"] = json!(s);
    }
    if let Some(b) = p["notify_sound"].as_bool() {
        saved["notify_sound"] = json!(b);
    }
    if let Some(m) = p["notify_mode"].as_str() {
        if !NOTIFY_MODES.contains(&m) {
            return Err(axelstra_core::coded!("settings.bad_notify_mode", "notify_mode must be all, waiting or off"));
        }
        saved["notify_mode"] = json!(m);
    }
    if let Some(patch) = p.get("ui").filter(|v| !v.is_null()) {
        apply_ui_state(saved, patch)?;
    }
    Ok(())
}

const UI_MAX_KEYS: usize = 64;
const UI_MAX_VALUE: usize = 128 * 1024;
const UI_MAX_TOTAL: usize = 512 * 1024;

/// Merges a `settings.set` `ui` patch (`{key: string | null}`) into `saved["ui"]`: the choices the webview used to keep only
/// in its localStorage (pinned worktrees, folded groups, the notification list...), so they survive a changed WebView
/// profile or a reinstall. A null value removes the key. Nothing is applied unless the whole patch is valid.
fn apply_ui_state(saved: &mut Value, patch: &Value) -> Result<()> {
    let patch = patch.as_object().ok_or_else(|| axelstra_core::coded!("settings.ui_not_object", "ui must be an object of strings"))?;
    let mut ui = saved["ui"].as_object().cloned().unwrap_or_default();
    for (k, v) in patch {
        let key_ok =
            k.len() <= 64 && k.starts_with("axelstra.") && k.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
        if !key_ok {
            return Err(axelstra_core::coded!("settings.ui_key_not_allowed", "ui key {k:?} is not allowed", k = k));
        }
        match v {
            Value::Null => {
                ui.remove(k);
            }
            Value::String(s) if s.len() <= UI_MAX_VALUE => {
                ui.insert(k.clone(), v.clone());
            }
            _ => {
                return Err(axelstra_core::coded!(
                    "settings.ui_bad_value",
                    "ui value for {k:?} must be a string of at most {max} bytes",
                    k = k,
                    max = UI_MAX_VALUE
                ))
            }
        }
    }
    let total: usize = ui.values().filter_map(Value::as_str).map(str::len).sum();
    if ui.len() > UI_MAX_KEYS || total > UI_MAX_TOTAL {
        return Err(axelstra_core::coded!(
            "settings.ui_too_large",
            "ui state is too large ({keys} keys, {total} bytes)",
            keys = ui.len(),
            total = total
        ));
    }
    saved["ui"] = Value::Object(ui);
    Ok(())
}

/// The app-wide settings file next to the daemon state (missing or broken = no settings).
fn read_settings(saved: &std::path::Path) -> Value {
    read_json(&saved.with_file_name("settings.json")).ok().flatten().unwrap_or_else(|| json!({}))
}

/// Ephemeral worker: when it finishes (process exit, or quiet for `idle_secs` in "idle" mode) its last
/// output goes to the parent session as if typed, and the worker is closed so it stops using memory.
/// Typical use: a test run or a helper agent that reports back to the main agent and disappears.
/// How long a session that already called `axelstra done` must be quiet before it stops counting as busy.
const REPORTED_IDLE_MS: u64 = 10_000;

/// How long a terminal (not an agent) must have been quiet, with nothing running in it, before it no longer keeps its
/// worktree from being cleaned up.
const SHELL_IDLE_MS: u64 = 10 * 60_000;

static SWEEP_QUEUED: AtomicBool = AtomicBool::new(false);
/// A backoff retry of a pending removal is already waiting.
static RETRY_QUEUED: AtomicBool = AtomicBool::new(false);

/// Kicks the janitor a few seconds from now (once for a burst of events): after a `done` or a merge, so the finished
/// worktree goes away without waiting for anything else. The delay lets the reaper close the session; it is one
/// delay per event, not a loop.
fn sweep_soon(reg: &Arc<Registry>) {
    if SWEEP_QUEUED.swap(true, Ordering::SeqCst) {
        return;
    }
    let r = reg.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(4));
        SWEEP_QUEUED.store(false, Ordering::SeqCst);
        r.janitor.bump();
    });
}

/// Tells the main agent of the project that `item` (a sweep result with an `integrate.*` code) was left for a human or
/// for it. Typed into an agent, so it stays English.
fn tell_main_not_integrated(reg: &Arc<Registry>, item: &Value) {
    let Some(p) = crate::projects::load(&reg.store).into_iter().find(|p| item["project"].as_str() == Some(p.id.as_str())) else { return };
    let main = crate::projects::norm_path(&p.path);
    let target = {
        let metas = reg.metas.lock_recover();
        metas
            .iter()
            .find(|(_, m)| m["kind"] == "agent" && m["worktree"].as_str().is_some_and(|w| crate::projects::norm_path(w) == main))
            .map(|(id, _)| *id)
    };
    let Some((id, s)) = target.and_then(|id| reg.get(id).map(|s| (id, s))) else { return };
    let text = crate::prompts::not_integrated(
        item["path"].as_str().unwrap_or_default(),
        item["branch"].as_str().unwrap_or_default(),
        item["why"].as_str().unwrap_or_default(),
    );
    let m = reg.bus.send(None, id, "message", &text, None, None);
    reg.deliver_untrusted(id, s, &crate::msgbus::notice(&m, "axelstra"));
}

/// An agent that reported with `axelstra done` and then stays quiet for `REPORTED_IDLE_MS` is closed by itself (leads and
/// plain workers too; ephemeral ones have `spawn_reaper`): its process tree ends and its tab and record go. One the user
/// typed into after the report is theirs again and stays open. A session opened by hand that never said `done` is never
/// closed here. Woken by the pulse, one timer for the quiet spell.
fn spawn_done_closer(reg: Arc<Registry>, id: u32) {
    std::thread::spawn(move || {
        let armed = Instant::now();
        let mut seen = reg.pulse.now();
        loop {
            let Some(s) = reg.get(id) else { return };
            if s.try_wait().is_some() || reg.bus.finished(id).is_none() {
                return;
            }
            if s.since_input_ms() < armed.elapsed().as_millis() as u64 {
                return;
            }
            let idle = s.idle_ms();
            if idle >= REPORTED_IDLE_MS {
                break;
            }
            seen = reg.pulse.wait(seen, Some(Duration::from_millis(REPORTED_IDLE_MS - idle + 20)));
        }
        let Some(s) = reg.get(id) else { return };
        // a question asked after the report is still a question
        if reg.wait_of(id, s.since_input_ms(), s.idle_ms()).is_some() {
            return;
        }
        let wt = reg.metas.lock_recover().get(&id).and_then(|m| m["worktree"].as_str().map(String::from));
        reg.tear_down(&[s]);
        reg.forget_session(id);
        if let Some(w) = wt {
            reg.note_activity(&w);
        }
        reg.persist();
        reg.pulse.bump();
        reg.janitor.bump();
    });
}

fn spawn_reaper(reg: Arc<Registry>, id: u32, parent: Option<u32>, idle_mode: bool, idle_secs: u64, lines: usize) {
    std::thread::spawn(move || {
        let mut worked = false;
        let mut done_by_idle = false;
        let mut nudged = false;
        let mut seen = reg.pulse.now();
        loop {
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
                // it said it waits for an answer (hook event, ask, escalate): not finished, keep it. The parent was told
                // when the event came in.
                if reg.wait_of(id, s.since_input_ms(), idle).is_some() {
                    worked = false;
                } else if !nudged && reg.metas.lock_recover().get(&id).map(|m| m["kind"] == "agent").unwrap_or(false) {
                    // first quiet spell: ask the worker to report through the bus instead of scraping its screen
                    crate::msgbus::deliver(s.clone(), crate::msgbus::paste(crate::prompts::IDLE_NUDGE));
                    nudged = true;
                    worked = false;
                } else {
                    done_by_idle = true;
                    break;
                }
            }
            // no polling: the program exiting, `axelstra done`, output after a quiet spell and every request bump the
            // pulse; the one timer is the moment the session would count as idle for `idle_secs`
            let limit = idle_secs * 1000;
            let timeout = (idle_mode && worked && idle < limit).then(|| Duration::from_millis(limit - idle + 20));
            seen = reg.pulse.wait(seen, timeout);
        }
        let Some(s) = reg.get(id) else { return };
        // give the output pipe a moment to drain after the process ended
        std::thread::sleep(Duration::from_millis(300));
        let code = s.try_wait();
        let done_msg = reg.bus.finished(id);
        // what the terminal shows, not the escape-code soup of a TUI redrawing itself
        // for an agent TUI only its last reply, not the banner, tool-run lines and status bar around it
        let tail = axelstra_core::screen::last_reply(&s.screen_lines(200)).unwrap_or_else(|| s.screen_lines(lines).join("\n"));
        let label = reg.labels.lock_recover().get(&id).cloned().unwrap_or_else(|| crate::prompts::session_name(id));
        let status = crate::prompts::worker_status(done_msg.is_some(), done_by_idle, code);
        if let Some(parent) = parent.and_then(|p| reg.get(p).map(|s| (p, s))) {
            let agent = reg.metas.lock_recover().get(&parent.0).and_then(|m| m["kind"].as_str().map(|k| k == "agent")).unwrap_or(false);
            let body = crate::prompts::worker_body(done_msg.as_ref().map(|d| d["text"].as_str().unwrap_or_default()), &tail);
            let msg = crate::prompts::worker_report(&label, &status, &body);
            // an agent TUI takes a multi-line message as one paste; a plain shell gets the single line
            let data = if agent {
                crate::msgbus::paste_with(&msg, !reg.runs_in_bypass(parent.0))
            } else {
                crate::prompts::worker_echo(&crate::msgbus::echo_safe(&label), &crate::msgbus::echo_safe(&status)).into_bytes()
            };
            crate::msgbus::deliver(parent.1.clone(), data);
        }
        s.kill();
        // the report went to the parent above; now the session leaves the registry (and the watchers hear it)
        reg.retire(id, code);
    });
}

/// Kills the live sessions whose worktree folder no longer exists on disk (they would only keep using memory).
/// Sessions without a worktree are left alone. No timer: called when something may have removed a worktree (the
/// project list is asked for, the watched worktree vanished, a janitor sweep or an orchestration ended).
fn reap_orphans(reg: &Registry) {
    kill_sessions_where(reg, |w| !std::path::Path::new(w).exists());
}

/// Kills the live sessions whose worktree (from their meta) satisfies `pred`.
fn kill_sessions_where(reg: &Registry, pred: impl Fn(&str) -> bool) {
    let doomed: Vec<_> = {
        let sessions = reg.sessions.lock_recover();
        let metas = reg.metas.lock_recover();
        sessions
            .iter()
            .filter(|(_, s)| s.try_wait().is_none())
            .filter(|(id, _)| metas.get(id).and_then(|m| m["worktree"].as_str()).is_some_and(&pred))
            .map(|(_, s)| s.clone())
            .collect()
    };
    for s in doomed {
        s.kill();
    }
}

/// Starts the automations when they are due. No tick: it sleeps until the next one is due, or until the list changes
/// (`auto_wake`). Missed runs are not replayed.
fn spawn_scheduler(reg: Arc<Registry>) {
    tokio::spawn(async move {
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
        loop {
            let r = reg.clone();
            let next = tokio::task::spawn_blocking(move || {
                let path = crate::automations::store_path(&r.store);
                // the file is claimed under the lock; the (slow) worktree and agent start run after it is released,
                // so a UI save is never stuck behind a running automation
                let claimed: Vec<_> = {
                    let _g = r.auto_lock.lock_recover();
                    let list = crate::automations::load(&path);
                    crate::automations::due_now(&list, crate::automations::now_ts())
                        .into_iter()
                        .filter_map(|id| match claim_automation(&path, &id) {
                            Ok(a) => Some(a),
                            Err(e) => {
                                eprintln!("automation {id} failed: {e:#}");
                                None
                            }
                        })
                        .collect()
                };
                for a in claimed {
                    if let Err(e) = launch_automation(&r, &a) {
                        eprintln!("automation {} failed: {e:#}", a.id);
                    }
                }
                let _g = r.auto_lock.lock_recover();
                crate::automations::next_due_ts(&crate::automations::load(&path), crate::automations::now_ts())
            })
            .await
            .ok()
            .flatten();
            match next {
                Some(at) => {
                    // a clock that jumped (hibernation) makes the wait wrong at worst by one wake-up
                    let wait = Duration::from_secs((at - crate::automations::now_ts()).clamp(1, 24 * 3600) as u64);
                    tokio::select! {
                        _ = tokio::time::sleep(wait) => {}
                        _ = reg.auto_wake.notified() => {}
                    }
                }
                None => reg.auto_wake.notified().await,
            }
        }
    });
}

/// Reads the spending and reports level crossings. With no budget set it sleeps until the settings change
/// (`budget_wake`); with one it looks every two minutes, as often as the agents' usage logs it reads are worth
/// reading (they have no event to wait on).
fn spawn_budget_loop(reg: Arc<Registry>) {
    tokio::spawn(async move {
        loop {
            let r = reg.clone();
            let has = tokio::task::spawn_blocking(move || !misc::budget_check(&r).is_empty()).await.unwrap_or(false);
            if has {
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(120)) => {}
                    _ = reg.budget_wake.notified() => {}
                }
            } else {
                reg.budget_wake.notified().await;
            }
        }
    });
}

/// Tells the app when a file in the data directory that other processes write changes: `dispatch.json` is written by
/// the `axelstra dispatch` commands, which do not go through the daemon. Asleep until the OS reports a write.
fn spawn_data_watch(reg: Arc<Registry>) {
    use notify::{RecursiveMode, Watcher};
    let Some(dir) = reg.saved.parent().map(|d| d.to_path_buf()) else { return };
    std::thread::spawn(move || {
        let (tx, rx) = std::sync::mpsc::channel();
        let Ok(mut w) = notify::recommended_watcher(tx) else { return };
        if w.watch(&dir, RecursiveMode::NonRecursive).is_err() {
            return;
        }
        while let Ok(ev) = rx.recv() {
            let hit = |ev: &notify::Result<notify::Event>| {
                ev.as_ref().is_ok_and(|e| {
                    // reading the file (inotify reports it) is not a change
                    !matches!(e.kind, notify::EventKind::Access(_))
                        && e.paths.iter().any(|p| p.file_name().is_some_and(|n| n == "dispatch.json"))
                })
            };
            if !hit(&ev) {
                continue;
            }
            // a write is several events (temp file, rename): one announcement for the burst
            std::thread::sleep(Duration::from_millis(150));
            while rx.try_recv().is_ok() {}
            reg.broadcast("dispatch.changed");
        }
    });
}

/// Removes the worktrees whose work is merged (see `janitor`) when something may have made one removable: at start,
/// when a session ends (its worktree is no longer busy), after a merge, a `done`, a finished flow or orchestration.
/// Kicks that come close together make one sweep, at most one a minute. Besides the events there is one timer: the
/// earliest moment a hand-made worktree stops counting as touched (`MANUAL_QUIET_SECS` after its last session left),
/// and a slow tick (30 min) for what raises no local event, like a PR merged on GitHub. `janitor.off` turns it off.
/// Wakes the janitor again after the backoff delay of attempt `attempts` (one waiting timer at a time).
fn queue_janitor_retry(reg: &Arc<Registry>, attempts: u32) {
    if RETRY_QUEUED.swap(true, Ordering::SeqCst) {
        return;
    }
    let r = reg.clone();
    let delay = crate::janitor::retry_delay(attempts);
    std::thread::spawn(move || {
        std::thread::sleep(delay);
        RETRY_QUEUED.store(false, Ordering::SeqCst);
        r.janitor.bump();
    });
}

fn spawn_janitor(reg: Arc<Registry>) {
    let gap = reg.timing.gap;
    std::thread::spawn(move || {
        let mut seen = 0;
        let mut last: Option<Instant> = None;
        let mut attempts = 0u32;
        loop {
            // events wake it; the timer is the next moment something becomes removable by time alone, or the slow tick
            seen = reg.janitor.wait(seen, Some(reg.next_janitor_wake()));
            if let Some(left) = last.map(|l| gap.saturating_sub(l.elapsed())).filter(|d| !d.is_zero()) {
                // the kicks of the next minute are folded into this sweep
                std::thread::sleep(left);
                seen = reg.janitor.now();
            }
            last = Some(Instant::now());
            if reg.closing.load(Ordering::SeqCst) {
                continue;
            }
            // worktrees of a removed project whose folder was held: the user asked for them to go, so this runs even
            // with the automatic cleanup off
            let (retried, held) = if crate::removal::has_pending(&reg.store) {
                crate::removal::retry(&reg.store, &|path| reg.close_worktree(path))
            } else {
                (0, 0)
            };
            if retried > 0 {
                reg.projects_changed();
            }
            if reg.saved.with_file_name("janitor.off").exists() {
                if held > 0 {
                    attempts += 1;
                    queue_janitor_retry(&reg, attempts);
                } else {
                    attempts = 0;
                }
                continue;
            }
            // a session waiting to be restored still needs its worktree
            let busy = |path: &str| reg.worktree_busy(path) || reg.worktree_pending(path);
            let close = |path: &str| reg.close_worktree(path);
            let touched = |path: &str| reg.worktree_touched(path);
            let env = crate::janitor::Env { busy: &busy, close: &close, touched: &touched };
            let swept = crate::janitor::sweep_with(&reg.store, &env, false);
            for s in swept.iter().chain(reg.clean_leftovers(false).iter()) {
                eprintln!("janitor: {s}");
                // finished work that could not be integrated is also reported to the project's main agent
                if s["code"].as_str().is_some_and(|c| c.starts_with("integrate.")) {
                    tell_main_not_integrated(&reg, s);
                }
            }
            if swept.iter().any(|s| s["removed"] == true) {
                reap_orphans(&reg);
                // the sessions saved for the removed worktrees go from sessions.json now
                reg.persist();
                reg.projects_changed();
            }
            // a folder nothing we watch holds was left whole: look again with a growing, bounded delay (see
            // `janitor::retry_delay`); a session that exits kicks the janitor at once, so no fixed timer runs
            if held > 0 || swept.iter().any(|s| s["pending"] == true) {
                attempts += 1;
                queue_janitor_retry(&reg, attempts);
            } else {
                attempts = 0;
            }
            reg.announce_events();
        }
    });
}

/// Tells watching connections (`session.changed`, see `ensure_session_notifier`) when the state of any session
/// changed: started, exited, went quiet or working again, escalated, got a message... Woken by those events (`pulse`);
/// its only timer is the moment the earliest working session would count as quiet, and with every session quiet it
/// sleeps until the next event. An agent that just went quiet or ended has its screen saved, so a daemon that is
/// killed or crashes still leaves the restored tab its history.
fn spawn_session_monitor(reg: Arc<Registry>) {
    use axelstra_core::pty::QUIET_MS;
    std::thread::spawn(move || {
        let mut seen = reg.pulse.now();
        // (id, running, quiet, escalated, unread, finished, awaiting)
        let mut last: Vec<(u32, bool, bool, bool, usize, bool, bool)> = vec![];
        loop {
            let mut next: Option<u64> = None;
            let now: Vec<_> = {
                let sessions = reg.sessions.lock_recover();
                let mut ids: Vec<_> = sessions.keys().copied().collect();
                ids.sort();
                ids.into_iter()
                    .map(|id| {
                        let s = &sessions[&id];
                        let (running, idle) = (s.try_wait().is_none(), s.idle_ms());
                        if running && idle < QUIET_MS {
                            next = Some(next.map_or(QUIET_MS - idle, |n| n.min(QUIET_MS - idle)));
                        }
                        (
                            id,
                            running,
                            idle >= QUIET_MS,
                            reg.bus.is_escalated(id),
                            reg.bus.unread(id),
                            reg.bus.finished(id).is_some(),
                            reg.awaiting_of(id, s.view().since_input_ms(), idle).is_some(),
                        )
                    })
                    .collect()
            };
            if now != last {
                // working before, quiet or ended now
                let settled: Vec<u32> =
                    now.iter().filter(|n| (!n.1 || n.2) && last.iter().any(|l| l.0 == n.0 && l.1 && !l.2)).map(|n| n.0).collect();
                last = now;
                reg.broadcast("session.changed");
                save_histories_of(&reg, &settled);
            }
            seen = reg.pulse.wait(seen, next.map(|ms| Duration::from_millis(ms + 20)));
        }
    });
}

/// Fresh worktree + agent session with the automation's prompt. Records the start first so a failure never loops.
/// Marks the automation as started now and returns it. The caller holds `auto_lock`.
fn claim_automation(path: &std::path::Path, id: &str) -> Result<crate::automations::Automation> {
    let mut list = crate::automations::load(path);
    let a = list
        .iter_mut()
        .find(|a| a.id == id)
        .ok_or_else(|| axelstra_core::coded!("automation.unknown", "unknown automation {id}", id = id))?;
    a.last_run = crate::automations::now_ts();
    let a = a.clone();
    crate::automations::save_all(path, &list)?;
    Ok(a)
}

fn run_automation(reg: &Arc<Registry>, path: &std::path::Path, id: &str) -> Result<Value> {
    let a = {
        let _g = reg.auto_lock.lock_recover();
        claim_automation(path, id)?
    };
    launch_automation(reg, &a)
}

/// Fresh worktree + agent session for a claimed automation. Takes no lock of its own.
fn launch_automation(reg: &Arc<Registry>, a: &crate::automations::Automation) -> Result<Value> {
    let pr = crate::projects::find(&reg.store, &a.project)?;
    let stamp = chrono::Local::now().format("%m%d-%H%M").to_string();
    let wt = crate::projects::worktree_create(&pr, &format!("auto-{}-{stamp}", a.name))?;
    // AXELSTRA_AGENT_CLI swaps the agent binary (a wrapper script, or a harmless stand-in in tests)
    let (program, args) = match std::env::var("AXELSTRA_AGENT_CLI") {
        Ok(cli) => crate::automations::raw_command(&cli, &a.prompt),
        Err(_) => {
            let ag = axelstra_core::agents::require(&a.agent)?;
            if ag.locate().is_none() {
                return Err(ag.missing_error());
            }
            crate::automations::agent_command(&a.agent, &a.prompt, axelstra_core::git::folder_trusted(std::path::Path::new(&wt.path)))?
        }
    };
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

/// True for the real `axelstra` executable. A test binary (`roundtrip-<hash>.exe`) or a renamed copy cannot serve
/// `statusline-tee`, so it must never be written into a status line that every session will run.
fn is_axelstra_exe(exe: &std::path::Path) -> bool {
    exe.file_stem().is_some_and(|s| s.eq_ignore_ascii_case("axelstra"))
}

/// The settings file passed to Claude as `--settings`: a hook that records the agent's current conversation id (it
/// changes on /clear), keyed by AXELSTRA_KEY, and a status line command that hands Claude's own numbers (context
/// window, 5 h and weekly limits) to the app and then runs the user's own status line, so nothing they see changes.
/// Without a real `axelstra` exe the status line is left out, and an existing file is not overwritten.
fn hooks_json(exe: Option<&std::path::Path>) -> Value {
    let mut hook = json!({"hooks": {"SessionStart": [{"hooks": [{"type": "command",
        "command": "[ -n \"$AXELSTRA_KEY\" ] || exit 0; mkdir -p \"$AXELSTRA_DIR/sid\" && cat > \"$AXELSTRA_DIR/sid/$AXELSTRA_KEY.json\""}]}]}});
    if let Some(exe) = exe.filter(|e| is_axelstra_exe(e)) {
        hook["statusLine"] = json!({"type": "command", "command": format!("\"{}\" statusline-tee", exe.display())});
    }
    hook
}

fn write_hooks_json(dir: &std::path::Path, exe: Option<&std::path::Path>) {
    let file = dir.join("hooks.json");
    if !exe.is_some_and(is_axelstra_exe) && file.exists() {
        return;
    }
    let _ = std::fs::write(file, hooks_json(exe).to_string());
}

/// Serves until the process dies. Fails if another daemon owns the pipe.
pub async fn serve(pipe: String, token: String) -> Result<()> {
    serve_with_store(pipe, token, crate::projects::store_path()).await
}

/// Same as `serve`, with an explicit projects file (tests must not touch the user's).
pub async fn serve_with_store(pipe: String, token: String, store: std::path::PathBuf) -> Result<()> {
    crate::proto::refuse_real_when_testing(&pipe, store.parent().unwrap_or(std::path::Path::new("")))?;
    origin::adopt_orphans();
    let reg = Arc::new(Registry::new(store, pipe.clone(), token.clone()));
    let _ = reg.me.set(Arc::downgrade(&reg));
    let dir = reg.saved.parent().map(std::path::Path::to_path_buf).unwrap_or_default();
    let _ = std::fs::create_dir_all(&dir);
    write_hooks_json(&dir, std::env::current_exe().ok().as_deref());
    spawn_scheduler(reg.clone());
    spawn_budget_loop(reg.clone());
    spawn_data_watch(reg.clone());
    spawn_session_monitor(reg.clone());
    // worktrees that exist now and carry no mark become manual, before the janitor may look at them
    if let Err(e) = crate::projects::migrate_marks(&reg.store) {
        eprintln!("janitor: mark migration failed: {e:#}");
    }
    // the daemon starting is one of the moments to look for merged worktrees
    reg.janitor.bump();
    spawn_janitor(reg.clone());
    spawn_stall_watchdog(reg.clone());
    triggers::spawn_loop(reg.clone());
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
        let peer = origin::pipe_peer(&conn);
        tokio::spawn(handle(conn, reg.clone(), token.clone(), peer));
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
            return Err(axelstra_core::coded!("daemon.already_running", "another daemon is already listening on {path}", path = path));
        }
        std::fs::remove_file(path)?;
    }
    let listener = tokio::net::UnixListener::bind(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    loop {
        match listener.accept().await {
            Ok((conn, _)) => {
                let peer = origin::socket_peer(&conn);
                tokio::spawn(handle(conn, reg.clone(), token.clone(), peer));
            }
            Err(e) => {
                eprintln!("accept failed: {e}");
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    }
}

async fn read_frame(rd: &mut (impl AsyncReadExt + Unpin), max: usize) -> std::io::Result<(u8, Vec<u8>)> {
    let mut len = [0u8; 4];
    rd.read_exact(&mut len).await?;
    let len = u32::from_le_bytes(len) as usize;
    if len == 0 || len > max {
        return Err(std::io::ErrorKind::InvalidData.into());
    }
    let mut kind = [0u8; 1];
    rd.read_exact(&mut kind).await?;
    let mut buf = vec![0u8; len - 1];
    rd.read_exact(&mut buf).await?;
    Ok((kind[0], buf))
}

/// Keeps a stray daemon from lingering: with no connection, no running session and no enabled automation for
/// ten minutes (`AXELSTRA_IDLE_EXIT_SECS`, 0 turns it off) it exits. The app normally stops it on quit.
/// No polling: it looks again only when something happened (`pulse`: a connection, a session exiting, a request),
/// and the one timer is the exit deadline, armed while idle.
fn spawn_idle_exit(reg: Arc<Registry>) {
    let limit: u64 = std::env::var("AXELSTRA_IDLE_EXIT_SECS").ok().and_then(|v| v.parse().ok()).unwrap_or(600);
    if limit == 0 {
        return;
    }
    let limit = Duration::from_secs(limit);
    std::thread::spawn(move || {
        let mut seen = reg.pulse.now();
        let mut idle_since: Option<Instant> = None;
        loop {
            let busy = reg.clients.load(Ordering::SeqCst) > 0
                || reg.sessions.lock_recover().values().any(|s| s.try_wait().is_none())
                || crate::automations::load(&crate::automations::store_path(&reg.store)).iter().any(|a| a.enabled);
            if busy {
                idle_since = None;
                seen = reg.pulse.wait(seen, None);
                continue;
            }
            let left = limit.saturating_sub(idle_since.get_or_insert_with(Instant::now).elapsed());
            if left.is_zero() {
                reg.persist_now();
                std::process::exit(0);
            }
            seen = reg.pulse.wait(seen, Some(left));
        }
    });
}

struct ClientGuard(Arc<Registry>);

impl Drop for ClientGuard {
    fn drop(&mut self) {
        self.0.clients.fetch_sub(1, Ordering::SeqCst);
        self.0.pulse.bump();
    }
}

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
/// Before `hello` a peer may send small frames only: nobody unauthenticated gets to make the daemon allocate 16 MiB.
const PREAUTH_MAX_FRAME: usize = 64 << 10;
/// JSON requests parsed but not yet run on one connection; a client that sends faster waits (backpressure).
const CALL_QUEUE: usize = 256;
/// Keystrokes waiting for their session on one connection.
const INPUT_QUEUE: usize = 1024;
/// How long a full session input backlog is waited for before the keys are given up on (and the client told).
const INPUT_PATIENCE: Duration = Duration::from_secs(3);

/// What one connection accumulates over its life; the call worker owns it while a call runs.
#[derive(Default)]
struct ConnState {
    authed: bool,
    /// set by a `hello` that says it comes from the CLI (what agents run); such a connection cannot switch screen control on
    cli: bool,
    /// set by a `hello` that presented a session's capability token: such a connection has role=agent, and is that session
    agent: Option<u32>,
    /// process id of the client end, where the OS tells it: `hello` classifies the connection by it (see `origin`)
    peer: Option<u32>,
    subs: HashMap<u32, Arc<AtomicBool>>,
}

/// Runs one JSON call on the blocking pool and queues the reply (and then the deferred work) on `tx`.
async fn run_call(req: Req, reg: &Arc<Registry>, tx: &Tx, st: &Arc<Mutex<ConnState>>, token: &str) {
    let t0 = std::time::Instant::now();
    let (reg2, tx2, st2, token2) = (reg.clone(), tx.clone(), st.clone(), token.to_string());
    // dispatch does blocking work (git, process spawns, files): it runs on the blocking pool, so the connection's
    // reader (and with it every terminal's keystrokes) never waits for it. A bug in one handler must not take every
    // terminal down with the daemon.
    let joined = tokio::task::spawn_blocking(move || {
        let mut after: Vec<Box<dyn FnOnce() + Send>> = Vec::new();
        let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut guard = st2.lock_recover();
            let c = &mut *guard;
            dispatch(&mut Ctx {
                req: &req,
                p: &req.params,
                reg: &reg2,
                tx: &tx2,
                subs: &mut c.subs,
                authed: &mut c.authed,
                cli: &mut c.cli,
                agent: &mut c.agent,
                peer: c.peer,
                token: &token2,
                after: &mut after,
            })
        }))
        .unwrap_or_else(|_| {
            Err(axelstra_core::coded!("daemon.internal_error", "internal daemon error in {method} (see crash.log)", method = req.method))
        });
        (req, res, after)
    })
    .await;
    let Ok((req, res, after)) = joined else { return };
    // what an agent connection asked for is kept for the user to review
    let agent = st.lock_recover().agent;
    if let Some(me) = agent {
        reg.audit.record(me, &req.method, &req.params, &res.as_ref().map(|_| ()).map_err(|e| format!("{e:#}")));
    }
    // calls that wait on purpose are not slow
    if !matches!(req.method.as_str(), "ask" | "msg.ask" | "session.wait" | "hello") {
        crate::lag::note(&format!("daemon {}", req.method), t0.elapsed(), 30);
    }
    let body = match res {
        Ok(v) => json!({"id": req.id, "result": v}),
        Err(e) => {
            let mut b = json!({"id": req.id, "error": format!("{e:#}")});
            // additive: an old frontend keeps showing `error`, a new one translates `code` with `params`
            if let Some(c) = axelstra_core::msg::find(&e) {
                b["code"] = json!(c.code);
                b["params"] = c.params_json();
            }
            b
        }
    };
    let _ = tx.send(encode(JSON, body.to_string().as_bytes())).await;
    // deferred work (stream start) runs only after the reply is queued
    for f in after {
        f();
    }
    // a request may have changed what sessions show (a message, a hint, a removal): the monitor looks once
    reg.pulse.bump();
}

/// Writes one chunk of keystrokes; a full backlog (the program is not reading) is waited out for a while rather than
/// dropping keys at once. Returns false when the keys had to be given up on.
fn write_input(s: &Session, data: &[u8]) -> bool {
    let end = std::time::Instant::now() + INPUT_PATIENCE;
    loop {
        match s.write(data) {
            Ok(()) => return true,
            Err(_) if s.try_wait().is_some() => return false,
            Err(_) if std::time::Instant::now() >= end => return false,
            Err(_) => std::thread::sleep(Duration::from_millis(5)),
        }
    }
}

async fn handle(pipe: impl AsyncRead + AsyncWrite + Send + 'static, reg: Arc<Registry>, token: String, peer: Option<u32>) {
    reg.clients.fetch_add(1, Ordering::SeqCst);
    reg.pulse.bump();
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

    let st = Arc::new(Mutex::new(ConnState { peer, ..Default::default() }));
    let mut authed = false;
    // calls run in order on one worker (a client may pipeline `spawn` then `attach`), but off the reader
    let (call_tx, mut call_rx) = mpsc::channel::<Req>(CALL_QUEUE);
    let worker = {
        let (reg, tx, st, token) = (reg.clone(), tx.clone(), st.clone(), token.clone());
        tokio::spawn(async move {
            while let Some(req) = call_rx.recv().await {
                run_call(req, &reg, &tx, &st, &token).await;
            }
        })
    };
    // keystrokes have their own ordered worker, so a program that does not read its input delays only this queue
    let (in_tx, mut in_rx) = mpsc::channel::<(u32, Vec<u8>)>(INPUT_QUEUE);
    // the session this connection is, when it is an agent's (id + 1; 0 for the app and the CLI of the user). It is set by
    // the handshake before any input frame is read, and lives outside `st` because a running call holds that lock.
    let who = Arc::new(AtomicU64::new(0));
    let input_worker = {
        let (reg, tx, who) = (reg.clone(), tx.clone(), who.clone());
        tokio::spawn(async move {
            while let Some((id, data)) = in_rx.recv().await {
                let Some(s) = reg.get(id) else { continue };
                let me = who.load(Ordering::SeqCst);
                // an agent types only into itself and the sessions it started: keys are how a prompt gets approved
                let allowed = me == 0 || agent_drives(&reg, (me - 1) as u32, id);
                let t0 = std::time::Instant::now();
                let ok = allowed && tokio::task::spawn_blocking(move || write_input(&s, &data)).await.unwrap_or(false);
                crate::lag::note("daemon pty write", t0.elapsed(), 20);
                if !ok {
                    // the client learns that keys were lost instead of typing into the void
                    let ev = json!({"event": "session.input_dropped", "id": id});
                    let _ = tx.send(encode(EVENT, ev.to_string().as_bytes())).await;
                }
            }
        })
    };

    loop {
        // a connection that has not sent a valid `hello` yet must do so quickly, or it holds a task for nothing
        let frame = if authed {
            read_frame(&mut rd, MAX_FRAME).await
        } else {
            match tokio::time::timeout(HANDSHAKE_TIMEOUT, read_frame(&mut rd, PREAUTH_MAX_FRAME)).await {
                Ok(f) => f,
                Err(_) => break,
            }
        };
        let Ok((kind, payload)) = frame else { break };
        match kind {
            JSON => {
                let Ok(req) = serde_json::from_slice::<Req>(&payload) else { break };
                if !authed {
                    if req.method != "hello" {
                        break;
                    }
                    // the handshake runs inline: nothing else on this connection is trusted until it is done
                    run_call(req, &reg, &tx, &st, &token).await;
                    if !st.lock_recover().authed {
                        break;
                    }
                    if let Some(me) = st.lock_recover().agent {
                        who.store(u64::from(me) + 1, Ordering::SeqCst);
                    }
                    authed = true;
                } else if call_tx.send(req).await.is_err() {
                    break;
                }
            }
            INPUT if authed => {
                if let Some((id, data)) = split_session(&payload) {
                    if in_tx.send((id, data.to_vec())).await.is_err() {
                        break;
                    }
                }
            }
            _ => break,
        }
    }
    // client gone: stop forwarders once the queued calls are done (a call may still add one); sessions keep running
    drop(call_tx);
    drop(in_tx);
    tokio::spawn(async move {
        let _ = worker.await;
        let _ = input_worker.await;
        for stop in st.lock_recover().subs.values() {
            stop.store(true, Ordering::SeqCst);
        }
    });
}

/// Kills the sessions (whole process trees) and waits, up to `limit`, until each one is gone.
fn kill_and_wait(sessions: &[Arc<Session>], limit: Duration) {
    for s in sessions {
        s.kill();
    }
    let end = std::time::Instant::now() + limit;
    while std::time::Instant::now() < end && sessions.iter().any(|s| s.try_wait().is_none()) {
        std::thread::sleep(Duration::from_millis(25));
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

/// Lets a watching client hear when the state of any session changed (started, exited, went quiet, escalated, got a
/// message...) and when the worktrees changed behind its back, so the UI does not poll. The events come from
/// `spawn_session_monitor` and `projects_changed`; the client lists sessions itself when it connects.
fn ensure_session_notifier(reg: &Arc<Registry>, tx: &Tx, subs: &mut HashMap<u32, Arc<AtomicBool>>) {
    const KEY: u32 = u32::MAX - 1;
    if subs.contains_key(&KEY) {
        return;
    }
    let stop = Arc::new(AtomicBool::new(false));
    subs.insert(KEY, stop.clone());
    let mut w = reg.watchers.lock_recover();
    // connections that went away since the last event leave the list here too
    w.retain(|(s, _)| !s.load(Ordering::SeqCst));
    w.push((stop, tx.clone()));
}

/// Saves the screens of these sessions (agents with a history key only), for the next start to show dimmed.
fn save_histories_of(reg: &Registry, ids: &[u32]) {
    if ids.is_empty() {
        return;
    }
    let picked: Vec<(String, Arc<Session>)> = {
        let sessions = reg.sessions.lock_recover();
        let metas = reg.metas.lock_recover();
        ids.iter().filter_map(|id| Some((metas.get(id)?["key"].as_str()?.to_string(), sessions.get(id)?.clone()))).collect()
    };
    for (key, s) in picked {
        save_history(reg, &key, &s.screen_lines(300));
    }
}

/// The task file of an agent session, if it has one and the path is still one of ours.
fn note_path(reg: &Registry, id: u32) -> Option<std::path::PathBuf> {
    let note = std::path::PathBuf::from(reg.metas.lock_recover().get(&id)?["note"].as_str()?);
    let root = crate::notes::root(reg.saved.parent()?);
    crate::notes::inside_root(&root, &note).then_some(note)
}

type Picked = (u32, Arc<Session>, axelstra_core::pty::View, Option<String>, Option<Value>);

/// Parents of the sessions that still run for them: a parent waiting on live children is not stalled.
fn busy_parents(reg: &Registry, picked: &[Picked]) -> std::collections::HashSet<u32> {
    picked
        .iter()
        .filter(|(id, s, ..)| s.try_wait().is_none() && reg.bus.finished(*id).is_none())
        .filter_map(|(_, _, _, _, m)| m.as_ref().and_then(|m| m["parent"].as_u64()).map(|p| p as u32))
        .collect()
}

/// What the stall watchdog needs to know about a session (see `stall::is_stalled`).
fn stall_facts(
    reg: &Registry,
    id: u32,
    running: bool,
    idle_ms: u64,
    since_input_ms: u64,
    meta: Option<&Value>,
    busy: &std::collections::HashSet<u32>,
) -> crate::stall::Facts {
    crate::stall::Facts {
        agent: meta.is_some_and(|m| m["kind"] == "agent"),
        main: meta.is_some_and(|m| m["main"] == true),
        running,
        reported: reg.bus.finished(id).is_some(),
        escalated: reg.bus.is_escalated(id),
        awaiting: reg.awaiting_of(id, since_input_ms, idle_ms).is_some(),
        live_children: busy.contains(&id),
        idle_ms,
        since_input_ms,
    }
}

/// Every 10 s: an agent that became stalled gets one message to its parent (until it works again).
fn spawn_stall_watchdog(reg: Arc<Registry>) {
    std::thread::spawn(move || loop {
        // tests shorten the threshold to seconds and then need a faster pass too
        std::thread::sleep(if std::env::var_os("AXELSTRA_STALL_SECS").is_some() {
            Duration::from_millis(300)
        } else {
            Duration::from_secs(10)
        });
        stall_tick(&reg);
    });
}

/// One pass of the watchdog.
fn stall_tick(reg: &Arc<Registry>) {
    let threshold = crate::stall::threshold_ms(&read_settings(&reg.saved));
    let minutes = crate::stall::display_minutes(threshold);
    let picked: Vec<Picked> = {
        let l = reg.sessions.lock_recover();
        let labels = reg.labels.lock_recover();
        let metas = reg.metas.lock_recover();
        l.iter().map(|(id, s)| (*id, s.clone(), s.view(), labels.get(id).cloned(), metas.get(id).cloned())).collect()
    };
    let busy = busy_parents(reg, &picked);
    let mut stalled = Vec::new();
    for (id, s, view, label, meta) in &picked {
        let facts = stall_facts(reg, *id, s.try_wait().is_none(), view.idle_ms(), view.since_input_ms(), meta.as_ref(), &busy);
        if crate::stall::is_stalled(&facts, threshold) {
            let parent = meta.as_ref().and_then(|m| m["parent"].as_u64()).map(|p| p as u32);
            stalled.push((*id, label.clone(), parent, view.screen_tail(12)));
        }
    }
    let ids: std::collections::HashSet<u32> = stalled.iter().map(|x| x.0).collect();
    let fresh: Vec<_> = {
        let mut seen = reg.stall_notified.lock_recover();
        seen.retain(|id| ids.contains(id));
        stalled.into_iter().filter(|x| seen.insert(x.0)).collect()
    };
    for (id, label, parent, screen) in fresh {
        let Some((pid, ps)) = parent.and_then(|p| reg.get(p).map(|s| (p, s))) else { continue };
        let label = label.unwrap_or_else(|| format!("session {id}"));
        let text = crate::stall::parent_text(&label, id, minutes, &crate::stall::last_line(&screen));
        let m = reg.bus.send(Some(id), pid, "message", &text, None, None);
        let agent = reg.metas.lock_recover().get(&pid).map(|m| m["kind"] == "agent").unwrap_or(false);
        if agent {
            reg.deliver_untrusted(pid, ps, &crate::msgbus::notice(&m, &format!("{label} ({})", crate::prompts::session_name(id))));
        }
    }
}

/// What `session.list` answers. Takes sessions, labels, metas in the registry's lock order.
fn session_list(reg: &Registry) -> Vec<Value> {
    // the locks only pick the sessions (cheap views and clones); screen rendering and the waiting heuristics run after
    let picked: Vec<_> = {
        let l = reg.sessions.lock_recover();
        let labels = reg.labels.lock_recover();
        let metas = reg.metas.lock_recover();
        l.iter().map(|(id, s)| (*id, s.clone(), s.view(), labels.get(id).cloned(), metas.get(id).cloned())).collect()
    };
    let stall_ms = crate::stall::threshold_ms(&read_settings(&reg.saved));
    let stall_min = crate::stall::display_minutes(stall_ms);
    let busy = busy_parents(reg, &picked);
    let mut v: Vec<Value> = picked
        .into_iter()
        .map(|(id, s, view, label, meta)| {
            let running = s.try_wait().is_none();
            let idle = view.idle_ms();
            // waiting is what the agent said (ask, escalate, a hook event), never read off its screen
            let wait = if running { reg.wait_of(id, view.since_input_ms(), idle) } else { None };
            let waiting = wait.is_some();
            let awaiting = if running { reg.awaiting_of(id, view.since_input_ms(), idle) } else { None };
            let facts = stall_facts(reg, id, running, idle, view.since_input_ms(), meta.as_ref(), &busy);
            let stalled = crate::stall::is_stalled(&facts, stall_ms);
            let stall = stalled.then(|| {
                let line = crate::stall::last_line(&view.screen_tail(12));
                let task = label.clone().unwrap_or_else(|| format!("session {id}"));
                let c = axelstra_core::msg::Coded::new(
                    "agent.stalled",
                    format!("{task} has been stalled for {stall_min} min without reporting. Last line: {line}"),
                    &[("task", task.clone()), ("min", stall_min.to_string()), ("line", line)],
                );
                json!({"minutes": stall_min, "code": c.code, "params": c.params_json(), "text": c.to_string()})
            });
            json!({"id": id, "running": running, "idle_ms": idle, "waiting": waiting, "wait": wait.map(|w| json!({"kind": w.kind.as_str(), "text": w.text})), "awaiting": awaiting.map(|a| json!({"reason": a.reason, "since": a.since, "until": a.until})), "stalled": stalled, "stall": stall, "escalated": reg.bus.is_escalated(id), "unread": reg.bus.unread(id), "finished": reg.bus.finished(id).is_some(), "label": label, "meta": meta})
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

/// One JSON call as the handlers see it: the request, the registry and the state of the connection it came in on.
struct Ctx<'a> {
    req: &'a Req,
    p: &'a Value,
    reg: &'a Arc<Registry>,
    tx: &'a Tx,
    subs: &'a mut HashMap<u32, Arc<AtomicBool>>,
    authed: &'a mut bool,
    cli: &'a mut bool,
    agent: &'a mut Option<u32>,
    peer: Option<u32>,
    token: &'a str,
    after: &'a mut Vec<Box<dyn FnOnce() + Send>>,
}

/// Whether `method` changes projects or worktrees, which makes the cached project list stale.
fn changes_projects(method: &str) -> bool {
    (method.starts_with("worktree.") && !matches!(method, "worktree.status" | "worktree.overview"))
        || (method.starts_with("project.")
            && !matches!(
                method,
                "project.list"
                    | "project.logo"
                    | "project.log"
                    | "project.discover"
                    | "project.clone"
                    | "project.clone_status"
                    | "project.clone_cancel"
            ))
}

/// Drops the cached project list and bumps the generation a `project.list` in flight checks before caching its answer.
fn forget_project_list(reg: &Registry) {
    *reg.plist.lock_recover() = None;
    crate::overview::invalidate();
    reg.wt_gen.fetch_add(1, Ordering::SeqCst);
}

fn dispatch(c: &mut Ctx) -> Result<Value> {
    // Forgotten before the call and again after it: a `project.list` that started in between (the UI polls it) would
    // otherwise cache the state from before the change and serve it for the next seconds.
    let (reg, mutating) = (c.reg.clone(), changes_projects(&c.req.method));
    if mutating {
        forget_project_list(&reg);
    }
    let out = dispatch_call(c);
    if mutating {
        forget_project_list(&reg);
        // a change made from the CLI or a flow shows up in the app without it polling (once the reply is queued)
        let r = reg.clone();
        c.after.push(Box::new(move || r.broadcast("projects.changed")));
    }
    out
}

fn dispatch_call(c: &mut Ctx) -> Result<Value> {
    let (req, p) = (c.req, c.p);
    if c.agent.is_some() && agent_forbidden(&req.method, p) {
        return Err(axelstra_core::coded!(
            "rpc.agent_forbidden",
            "{method} is not allowed to agents: it is done by the user in the app",
            method = req.method
        ));
    }
    // an agent reads its own inbox and writes only its own task file: its credential names the session
    if let (Some(me), "msg.inbox" | "notes.append") = (*c.agent, req.method.as_str()) {
        if p["session"].as_u64() != Some(u64::from(me)) {
            return Err(axelstra_core::coded!(
                "rpc.agent_own_session",
                "{method} is only allowed for the agent's own session",
                method = req.method
            ));
        }
    }
    if sources::handles(&req.method) {
        return sources::call(c);
    }
    match req.method.split('.').next().unwrap_or_default() {
        "session" => session::call(c),
        "worktree" | "project" | "pm" | "janitor" | "dispatch" | "ports" => worktrees::call(c),
        "fs" | "files" | "watch" | "open" => files::call(c),
        "git" | "gh" => vcs::call(c),
        "triggers" => triggers::call(c),
        "docker" => containers::call(c),
        "orch" | "kanban" | "automation" => orchestration::call(c),
        "msg" | "notes" => messaging::call(c),
        "agent" | "agents" | "codex" | "computer" | "settings" | "flow" | "app" | "hooks" | "ping" | "audit" | "notices" | "daemon"
        | "hello" | "backup" | "env" | "budget" | "workspace" | "host" => misc::call(c),
        _ => anyhow::bail!("unknown method {}", req.method),
    }
}

#[cfg(test)]
mod settings_tests {
    use super::*;

    #[test]
    fn ui_state_merges_removes_and_survives_other_patches() {
        let mut saved = json!({});
        apply_ui_settings(&mut saved, &json!({"ui": {"axelstra.pins": "[\"a\"]", "axelstra.groupsFolded": "[]"}})).unwrap();
        apply_ui_settings(&mut saved, &json!({"ui": {"axelstra.groupsFolded": null}, "shell": "wsl"})).unwrap();
        assert_eq!(saved["ui"], json!({"axelstra.pins": "[\"a\"]"}));
        apply_ui_settings(&mut saved, &json!({"notify_mode": "off"})).unwrap();
        assert_eq!(saved["ui"], json!({"axelstra.pins": "[\"a\"]"}));
    }

    #[test]
    fn invalid_ui_state_changes_nothing() {
        let mut saved = json!({"ui": {"axelstra.pins": "[]"}});
        for bad in [
            json!({"other.key": "x"}),
            json!({"axelstra.a b": "x"}),
            json!({"axelstra.pins": 5}),
            json!({"axelstra.pins": "ok", "axelstra.big": "x".repeat(UI_MAX_VALUE + 1)}),
        ] {
            assert!(apply_ui_settings(&mut saved, &json!({ "ui": bad })).is_err());
        }
        assert!(apply_ui_settings(&mut saved, &json!({"ui": ["x"]})).is_err());
        assert_eq!(saved["ui"], json!({"axelstra.pins": "[]"}));
        let many: serde_json::Map<String, Value> = (0..=UI_MAX_KEYS).map(|i| (format!("axelstra.k{i}"), json!("v"))).collect();
        assert!(apply_ui_settings(&mut saved, &json!({"ui": many})).is_err());
    }

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

    #[test]
    fn the_janitor_timer_is_the_next_moment_a_worktree_stops_being_touched_or_the_slow_tick() {
        let t = JanitorTiming { quiet: Duration::from_secs(600), gap: Duration::ZERO, slow: Duration::from_secs(1800) };
        let ms = |n| Duration::from_millis(n);
        // a daemon older than the quiet spell and no recent activity: only the slow tick
        assert_eq!(janitor_wake(&t, Duration::from_secs(700), std::iter::empty()), t.slow);
        // a fresh daemon: when it stops being too new to know
        assert_eq!(janitor_wake(&t, Duration::from_secs(100), std::iter::empty()), Duration::from_secs(500) + ms(20));
        // the session that left last decides, the older ones do not
        let left = [Duration::from_secs(550), Duration::from_secs(10), Duration::from_secs(900)];
        assert_eq!(janitor_wake(&t, Duration::from_secs(700), left.into_iter()), Duration::from_secs(50) + ms(20));
        // a slow tick shorter than the spell wins
        let short = JanitorTiming { slow: Duration::from_secs(30), ..t };
        assert_eq!(janitor_wake(&short, Duration::from_secs(1), std::iter::empty()), Duration::from_secs(30));
    }

    /// A worktree made by hand whose branch is merged goes once it has been quiet for the quiet spell, with no event at
    /// all in between: the janitor's own timer looks again.
    #[test]
    fn the_janitor_looks_again_after_the_quiet_spell_without_any_event() {
        let root = std::env::temp_dir().join(format!("axl-reg-jtimer-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let repo = root.join("proj");
        std::fs::create_dir_all(&repo).unwrap();
        let g = |dir: &std::path::Path, args: &[&str]| assert!(axelstra_core::git::run(dir, args).unwrap().ok, "git {args:?}");
        let commit = ["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "--allow-empty", "-m", "x"];
        g(&repo, &["init", "-q", "-b", "main"]);
        g(&repo, &commit);
        let store = root.join("projects.json");
        let p = crate::projects::add(&store, &repo.to_string_lossy()).unwrap();
        let wt = crate::projects::worktree_create(&p, "mine").unwrap();
        g(std::path::Path::new(&wt.path), &commit);
        g(&repo, &["merge", "-q", "--ff-only", "ax/mine"]);
        crate::projects::set_manual(&store, &p.id, &wt.path).unwrap();
        // the registry (and so the daemon's age) starts after the setup, with a short quiet spell
        let mut r = Registry::new(store, "p".into(), "t".into());
        r.timing = JanitorTiming { quiet: Duration::from_secs(3), gap: Duration::ZERO, slow: Duration::from_secs(3600) };
        let reg = Arc::new(r);
        reg.janitor.bump();
        spawn_janitor(reg.clone());
        std::thread::sleep(Duration::from_millis(1000));
        assert!(std::path::Path::new(&wt.path).exists(), "removed while the daemon was still too new to know");
        let t = Instant::now();
        while std::path::Path::new(&wt.path).exists() {
            assert!(t.elapsed() < Duration::from_secs(30), "the janitor never looked again");
            std::thread::sleep(Duration::from_millis(100));
        }
        reg.closing.store(true, Ordering::SeqCst);
        axelstra_core::git::purge_trash(std::path::Path::new(&wt.path).parent().unwrap());
        let _ = axelstra_core::git::remove_tree(&root);
    }

    /// A burst of saves is written once at the end of the spell, and the backup copies are not shifted each time.
    #[test]
    fn saves_in_a_burst_are_debounced_and_do_not_rotate_the_backups() {
        let dir = std::env::temp_dir().join(format!("axl-reg-save-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let reg = Arc::new(Registry::new(dir.join("projects.json"), "p".into(), "t".into()));
        let _ = reg.me.set(Arc::downgrade(&reg));
        let rec =
            |key: &str| json!({"program": "x", "cwd": dir.to_string_lossy(), "meta": {"worktree": dir.to_string_lossy(), "key": key}});
        let saved = || -> Vec<Value> {
            let text = std::fs::read_to_string(dir.join("sessions.json")).unwrap_or_default();
            serde_json::from_str::<Value>(&text).map(|v| v["data"].as_array().cloned().unwrap_or_default()).unwrap_or_default()
        };
        // the first save after a quiet spell is immediate
        reg.specs.lock_recover().insert(1, rec("k1"));
        reg.persist();
        assert_eq!(saved().len(), 1);
        // the next ones come within the spell: they wait, and become one write
        for i in 2..=6 {
            reg.specs.lock_recover().insert(i, rec(&format!("k{i}")));
            reg.persist();
        }
        assert_eq!(saved().len(), 1, "saved before the end of the spell");
        let t = Instant::now();
        while saved().len() < 6 {
            assert!(t.elapsed() < Duration::from_secs(10), "the last save never came");
            std::thread::sleep(Duration::from_millis(20));
        }
        // two writes happened in a row, and neither shifted a backup over the other
        assert!(!dir.join("sessions.json.1").exists(), "a backup of a few seconds ago");
        // an immediate save (shutdown) is written whatever the spell
        reg.specs.lock_recover().insert(7, rec("k7"));
        reg.persist_now();
        assert_eq!(saved().len(), 7);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Writes that race keep the newest snapshot on disk.
    #[test]
    fn racing_saves_leave_the_newest_state_on_disk() {
        let reg = temp_registry("racesave");
        let dir = reg.saved.parent().unwrap().to_path_buf();
        let rec =
            |key: &str| json!({"program": "x", "cwd": dir.to_string_lossy(), "meta": {"worktree": dir.to_string_lossy(), "key": key}});
        let mut hs = vec![];
        for t in 0..4 {
            let (r, rec) = (reg.clone(), rec(&format!("k{t}")));
            hs.push(std::thread::spawn(move || {
                r.specs.lock_recover().insert(t, rec);
                r.persist_now();
            }));
        }
        for h in hs {
            h.join().unwrap();
        }
        reg.persist_now();
        let text = std::fs::read_to_string(dir.join("sessions.json")).unwrap();
        assert_eq!(serde_json::from_str::<Value>(&text).unwrap()["data"].as_array().unwrap().len(), 4);
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
        let bad = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|p| p.file_name().unwrap().to_string_lossy().starts_with("sessions.json.bad."))
            .expect("corrupt file kept as .bad.<ts>");
        assert_eq!(std::fs::read_to_string(bad).unwrap(), "[{ broken");
        let saved: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("sessions.json")).unwrap()).unwrap();
        assert_eq!(saved, json!({"v": 1, "data": []}));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sessions_of_deleted_worktrees_are_pruned_at_start_after_a_backup() {
        let dir = std::env::temp_dir().join(format!("axl-reg-prune-{}", std::process::id()));
        let alive = dir.join("wt-alive");
        std::fs::create_dir_all(&alive).unwrap();
        let rec = |w: &std::path::Path| json!({"program": "x", "cwd": w.to_string_lossy(), "meta": {"worktree": w.to_string_lossy()}});
        let gone = dir.join("wt-gone");
        write_json(&dir.join("sessions.json"), &vec![rec(&alive), rec(&gone), rec(&gone)]).unwrap();
        let reg = Registry::new(dir.join("projects.json"), "p".into(), "t".into());
        assert_eq!(reg.pending.lock_recover().len(), 1);
        let saved: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("sessions.json")).unwrap()).unwrap();
        assert_eq!(saved["data"].as_array().unwrap().len(), 1);
        let kept = std::fs::read_dir(dir.join("backups")).unwrap().count();
        assert_eq!(kept, 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_persist_drops_records_of_deleted_worktrees_with_their_sid_files() {
        let dir = std::env::temp_dir().join(format!("axl-reg-persist-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let alive = dir.join("wt-alive");
        std::fs::create_dir_all(&alive).unwrap();
        std::fs::create_dir_all(dir.join("sid")).unwrap();
        let gone = dir.join("wt-gone");
        let rec = |w: &std::path::Path, key: &str| json!({"program": "x", "cwd": w.to_string_lossy(), "meta": {"worktree": w.to_string_lossy(), "key": key}});
        std::fs::write(dir.join("sid").join("aaa.json"), "{}").unwrap();
        std::fs::write(dir.join("sid").join("bbb.json"), "{}").unwrap();
        let reg = Registry::new(dir.join("projects.json"), "p".into(), "t".into());
        *reg.pending.lock_recover() = vec![rec(&alive, "aaa"), rec(&gone, "bbb")];
        reg.persist();
        assert_eq!(reg.pending.lock_recover().len(), 1);
        let saved: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("sessions.json")).unwrap()).unwrap();
        assert_eq!(saved["data"].as_array().unwrap().len(), 1);
        assert!(dir.join("sid").join("aaa.json").exists());
        assert!(!dir.join("sid").join("bbb.json").exists());
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
                for _ in 0..300 {
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

#[cfg(test)]
mod hooks_json_tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn only_the_real_axelstra_exe_becomes_the_status_line() {
        let status = |p: &str| hooks_json(Some(Path::new(p)))["statusLine"]["command"].as_str().map(String::from);
        if cfg!(windows) {
            assert_eq!(status(r"D:\x\axelstra.exe").as_deref(), Some(r#""D:\x\axelstra.exe" statusline-tee"#));
        }
        assert_eq!(status("/usr/bin/axelstra").as_deref(), Some(r#""/usr/bin/axelstra" statusline-tee"#));
        assert_eq!(status(r"C:\t\deps\roundtrip-d0504a754dee3ab3.exe"), None);
        assert_eq!(status(r"C:\t\deps\axelstra_daemon-1a2b.exe"), None);
        assert!(hooks_json(None)["statusLine"].is_null());
        assert!(hooks_json(None)["hooks"]["SessionStart"].is_array());
    }

    #[test]
    fn a_test_binary_does_not_clobber_the_hooks_file() {
        let dir = std::env::temp_dir().join(format!("axl-hj-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (real, test_exe) =
            if cfg!(windows) { (r"C:\x\axelstra.exe", r"C:\x\deps\roundtrip-1.exe") } else { ("/x/axelstra", "/x/deps/roundtrip-1") };
        write_hooks_json(&dir, Some(Path::new(real)));
        let good = std::fs::read_to_string(dir.join("hooks.json")).unwrap();
        assert!(good.contains("statusline-tee"));
        write_hooks_json(&dir, Some(Path::new(test_exe)));
        assert_eq!(std::fs::read_to_string(dir.join("hooks.json")).unwrap(), good);
        // first run from a non-axelstra exe still gets the session hook, just no status line
        let fresh = dir.join("fresh");
        std::fs::create_dir_all(&fresh).unwrap();
        write_hooks_json(&fresh, Some(Path::new(test_exe)));
        let t = std::fs::read_to_string(fresh.join("hooks.json")).unwrap();
        assert!(t.contains("SessionStart") && !t.contains("statusLine"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
