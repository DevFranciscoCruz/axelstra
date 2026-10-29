//! The daemon's registry of sessions and its helpers: the state every request and background loop shares.

use super::*;

/// Lock order. Hold at most these in this order, never a later one while taking an earlier one:
///   auto_lock, sessions, labels, metas, specs, pending, then the leaf locks (runs, flows, plist, watchers, the pulses).
/// Leaf locks are never held while taking another lock. A path that needs two takes them in this order
/// (`session.list`, `daemon.shutdown` and `persist` do); the `lock_order` test runs those paths against each other.
pub(super) struct Registry {
    pub(super) next: AtomicU32,
    pub(super) sessions: Mutex<HashMap<u32, Arc<Session>>>,
    pub(super) labels: Mutex<HashMap<u32, String>>,
    /// free-form UI data per session (project, worktree...), returned by session.list
    pub(super) metas: Mutex<HashMap<u32, Value>>,
    pub(super) store: std::path::PathBuf,
    /// where background orchestration runs connect back to this daemon
    pub(super) pipe: String,
    pub(super) token: String,
    /// per-session capability tokens (env AXELSTRA_AGENT_TOKEN): a connection that presents one has role=agent
    pub(super) agent_tokens: Mutex<HashMap<String, u32>>,
    /// latest orchestration run per project id
    pub(super) runs: Mutex<HashMap<String, Value>>,
    /// stop switch of the run that is going, per project id (`orch.cancel`)
    pub(super) run_cancels: Mutex<HashMap<String, axelstra_core::orch::Cancel>>,
    /// serializes read-modify-write of automations.json (UI saves vs the scheduler)
    pub(super) auto_lock: Mutex<()>,
    /// running `axelstra flow` plans: flow id -> [{id, worktree, state}] (what the sidebar shows as queued/running/done)
    pub(super) flows: Mutex<HashMap<String, Value>>,
    /// project.list answer reused for a few seconds (it starts several git processes per project)
    pub(super) plist: Mutex<Option<(std::time::Instant, Value)>>,
    /// the last project.list answer, whatever its age: served when a fresh one takes too long
    pub(super) plist_last: Mutex<Option<Value>>,
    /// spawn requests of live sessions, kept on disk so a daemon restart can bring them back
    pub(super) specs: Mutex<HashMap<u32, Value>>,
    /// sessions of the previous daemon run, waiting for the UI to restore (or discard) them
    pub(super) pending: Mutex<Vec<Value>>,
    pub(super) saved: std::path::PathBuf,
    /// messages between sessions, worker bookkeeping and the worker limit
    pub(super) bus: crate::msgbus::Bus,
    /// open connections (the app, CLI calls); a daemon nobody talks to ends itself, see `spawn_idle_exit`
    pub(super) clients: std::sync::atomic::AtomicUsize,
    /// agents may look at the screen and drive mouse and keyboard (`axelstra computer`); off at every start
    pub(super) computer: AtomicBool,
    /// what agents' own hooks said they are doing (`session.hint`)
    pub(super) hints: axelstra_core::waiting::Hints,
    /// the permission mode an agent's own hook last reported (`session.hint` `mode`), by session
    pub(super) modes: Mutex<HashMap<u32, String>>,
    /// sessions that said they only wait for something external (`session.await`)
    pub(super) awaits: axelstra_core::waiting::Awaits,
    /// sessions the stall watchdog already told their parent about (until they work again)
    pub(super) stall_notified: Mutex<HashMap<u32, std::time::Instant>>,
    /// work a quiet session has that no screen shows (processes it started, a wish for a worker slot)
    pub(super) work: crate::stall::Work,
    /// bumped when a request changed projects or worktrees; watchers hear it as `projects.changed`
    pub(super) wt_gen: AtomicU64,
    /// what agents asked the daemon to do (`audit.list`)
    pub(super) audit: crate::audit::Audit,
    /// `hello`s refused with a bad token since the last good one: each failure makes the next answer slower
    pub(super) bad_hellos: AtomicU32,
    /// bumped by anything that may change what `session.list` shows or whether the daemon is idle (a request, a
    /// connection coming or going, a session starting, exiting or printing after a quiet spell); see `spawn_session_monitor`
    pub(super) pulse: Pulse,
    /// the one scheduler of the reapers and done-closers (see `Reapers`)
    pub(super) reapers: Reapers,
    /// connections that get `session.changed` and `projects.changed` (their `watch.subscribe`), with their stop flag
    pub(super) watchers: Mutex<Vec<(Arc<AtomicBool>, Tx)>>,
    /// the automation list changed: the scheduler works out its next due time again
    pub(super) auto_wake: tokio::sync::Notify,
    /// the budgets in the settings changed: the budget loop reads them again
    pub(super) budget_wake: tokio::sync::Notify,
    /// something may have made a worktree removable (a session ended, a merge, a finished flow): see `spawn_janitor`
    pub(super) janitor: Pulse,
    /// `daemon.shutdown` is under way: the janitor must not start removing a worktree it could not finish
    pub(super) closing: AtomicBool,
    /// worktree (normalized) -> when a session last left it, for the janitor's "used a moment ago" rule
    pub(super) activity: Mutex<HashMap<String, Instant>>,
    /// when this daemon started: what it does not remember about a worktree it cannot rule out
    pub(super) started: Instant,
    /// what the cleanup closed so far: sessions, processes, bytes of RAM (the app reads the difference of a manual run)
    pub(super) closed: Closed,
    /// the last sessions that ended on their own and left the registry (`retire`): a leaf lock. `session.status` still
    /// answers for them, so `axelstra wait`, flows and the orchestrator get their exit code after the row is gone
    pub(super) exited: Mutex<std::collections::VecDeque<Exited>>,
    /// the janitor's time constants (tests shorten them)
    pub(super) timing: JanitorTiming,
    /// this registry as the background threads see it (set once the daemon serves): a save that is too soon after
    /// the last one is finished by a thread that needs it. Unset in unit tests, where a save is always immediate
    pub(super) me: std::sync::OnceLock<std::sync::Weak<Registry>>,
    /// when `sessions.json` was last saved, and whether a save is already waiting for its turn
    pub(super) save_turn: Mutex<SaveTurn>,
    /// the number of the last snapshot written and when the backups were last rotated; held while writing, so writes
    /// that race on disk keep the newest
    pub(super) save_written: Mutex<(u64, Option<Instant>)>,
    /// numbers the snapshots of the saved state (taken under the `specs`/`pending` locks)
    pub(super) save_gen: AtomicU64,
    /// settings.json as last read or written (the daemon is its only writer): a leaf lock. `settings_changed` drops it
    pub(super) settings: Mutex<Option<Value>>,
    /// hibernated sessions, wake queue, event counters (see `hibernate.rs`): a leaf lock
    pub(super) hib: crate::hibernate::Hib,
    /// `session.spawn` callers waiting in line for a worker slot or for memory (see `spawnq.rs`): a leaf lock
    pub(super) spawnq: Mutex<crate::spawnq::SpawnQ>,
    /// memory pressure (`crate::governor`); its thread starts with `server::governor::spawn`
    pub(super) governor: Arc<crate::governor::Governor>,
    /// what the governor froze (and the builds under `axelstra lock` it may freeze): see `server/pause.rs`
    pub(super) pauser: pause::Pauser,
}

#[derive(Default)]
pub(super) struct SaveTurn {
    pub(super) last: Option<Instant>,
    pub(super) queued: bool,
}

/// Saves of `sessions.json` closer together than this are one save, written at the end of the spell.
pub(super) const SAVE_DEBOUNCE: Duration = Duration::from_millis(250);
/// The rotating backups of `sessions.json` are shifted at most this often.
pub(super) const SAVE_ROTATE_EVERY: Duration = Duration::from_secs(5 * 60);

/// How long the janitor waits for things that become true with time, not with an event.
#[derive(Clone, Copy)]
pub(super) struct JanitorTiming {
    /// a hand-made worktree must have had no session for this long (`janitor::MANUAL_QUIET_SECS`)
    pub(super) quiet: Duration,
    /// kicks closer together than this make one sweep
    pub(super) gap: Duration,
    /// the one remote deadline, set only while a worktree waits to be integrated: a PR merged on GitHub raises no local
    /// event (a fetch that brings the merge does: see `prwatch::RefWatch`)
    pub(super) slow: Duration,
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
/// (the daemon being too new to know, or a session that left `elapsed` ago), or, only while a worktree with work of its
/// own waits to be integrated (`awaiting`: a PR merged on GitHub raises no local event), the one remote deadline `slow`.
/// `None`: nothing is due, it sleeps until an event.
pub(super) fn janitor_wake(t: &JanitorTiming, started: Duration, left: impl Iterator<Item = Duration>, awaiting: bool) -> Option<Duration> {
    let quiet = std::iter::once(started)
        .chain(left)
        .filter_map(|elapsed| t.quiet.checked_sub(elapsed).filter(|d| !d.is_zero()))
        .min()
        .map(|d| d + Duration::from_millis(20));
    match (quiet, awaiting.then_some(t.slow)) {
        (Some(q), Some(s)) => Some(q.min(s)),
        (q, s) => q.or(s),
    }
}

/// Whether a permission mode name (as an agent reports it) means no prompts.
pub(super) fn mode_is_bypass(mode: &str) -> bool {
    let m = mode.to_ascii_lowercase();
    m.contains("bypass") || m.contains("yolo") || m == "dontask" || m.contains("dangerously")
}

/// Whether an agent settings file makes bypass the default mode.
pub(super) fn settings_default_bypass(v: &Value) -> bool {
    v["permissions"]["defaultMode"].as_str().is_some_and(mode_is_bypass) || v["defaultMode"].as_str().is_some_and(mode_is_bypass)
}

/// How many ended sessions `Registry::exited` remembers.
pub(super) const EXITED_KEPT: usize = 256;

/// What is remembered of a session that ended and left the registry.
pub(super) struct Exited {
    pub(super) id: u32,
    pub(super) code: Option<i32>,
    /// it had reported with `axelstra done`
    pub(super) finished: bool,
}

/// Totals of what the janitor and the done-closer shut down.
#[derive(Default)]
pub(super) struct Closed {
    pub(super) sessions: AtomicU64,
    pub(super) procs: AtomicU64,
    pub(super) bytes: AtomicU64,
}

impl Closed {
    pub(super) fn read(&self) -> (u64, u64, u64) {
        (self.sessions.load(Ordering::SeqCst), self.procs.load(Ordering::SeqCst), self.bytes.load(Ordering::SeqCst))
    }
}

/// Wakes a background waiter when something happened, so it never has to poll.
#[derive(Default)]
pub(super) struct Pulse {
    pub(super) count: Mutex<u64>,
    pub(super) cv: Condvar,
}

impl Pulse {
    pub(super) fn bump(&self) {
        *self.count.lock_recover() += 1;
        self.cv.notify_all();
    }

    pub(super) fn now(&self) -> u64 {
        *self.count.lock_recover()
    }

    /// Waits for a bump after `seen` (the count it returned last time), or at most `timeout`. Returns the count now.
    pub(super) fn wait(&self, seen: u64, timeout: Option<Duration>) -> u64 {
        let g = self.count.lock_recover();
        let g = match timeout {
            None => self.cv.wait_while(g, |n| *n == seen).unwrap_or_else(PoisonError::into_inner),
            Some(t) => self.cv.wait_timeout_while(g, t, |n| *n == seen).unwrap_or_else(PoisonError::into_inner).0,
        };
        *g
    }
}

/// How a closed session without a label is named inside a coded message ("session {sessions} closed"): the word
/// "session" belongs to the message text in each language, so the daemon adds only the number.
pub(super) fn unnamed_label(id: u32) -> String {
    format!("#{id}")
}

impl Registry {
    /// Sends `{"event": name}` to every watching connection. One that stopped reading misses it rather than block us.
    pub(super) fn broadcast(&self, name: &str) {
        self.broadcast_event(json!({"event": name}));
    }

    /// Same, for an event that carries data (`session.exited`).
    pub(super) fn broadcast_event(&self, ev: Value) {
        let frame = encode(EVENT, ev.to_string().as_bytes());
        self.watchers.lock_recover().retain(|(stop, tx)| {
            !stop.load(Ordering::SeqCst) && !matches!(tx.try_send(frame.clone()), Err(mpsc::error::TrySendError::Closed(_)))
        });
    }

    /// `resources.changed`, with the machine's memory level and free memory riding along (additive).
    pub(super) fn broadcast_resources(&self) {
        let (level, info) = self.governor.state();
        self.broadcast_event(json!({"event": "resources.changed", "level": level.as_str(), "free_mb": info.free_mb}));
    }

    /// Tells the app there is something to take for its toasts and notification center (`janitor.events`,
    /// `triggers.events`, `budget.events`); it takes them with those calls, so it never asks on a timer.
    pub(super) fn announce_events(&self) {
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
    pub(super) fn projects_changed(&self) {
        forget_project_list(self);
        self.broadcast("projects.changed");
    }

    /// The settings, from memory once read. `f` runs under a leaf lock: it must not take another.
    pub(super) fn with_settings<R>(&self, f: impl FnOnce(&Value) -> R) -> R {
        f(self.settings.lock_recover().get_or_insert_with(|| read_settings(&self.saved)))
    }

    /// A copy of the settings.
    pub(super) fn settings(&self) -> Value {
        self.with_settings(Value::clone)
    }

    /// settings.json was written (`settings.set`, a project removal) or replaced (`backup.restore`, `workspace.import`):
    /// the next read goes to the disk.
    pub(super) fn settings_changed(&self) {
        *self.settings.lock_recover() = None;
    }

    /// Asks for sessions.json to be rewritten. The first save after a quiet spell is written at once; ones that follow
    /// within `SAVE_DEBOUNCE` become one save at the end of the spell (restoring N sessions saves twice, not N times).
    /// `persist_now` is the save itself, for the moments that must not wait (shutdown).
    pub(super) fn persist(&self) {
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
    pub(super) fn persist_now(&self) {
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
    pub(super) fn set_done(&self, id: u32, done: bool) {
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
    pub(super) fn clean_leftovers(&self, dry: bool) -> Vec<Value> {
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
    pub(super) fn drop_sid_of(&self, rec: &Value) {
        let Some(key) = rec["meta"]["key"].as_str().filter(|k| !k.is_empty() && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
        else {
            return;
        };
        let _ = std::fs::remove_file(self.saved.with_file_name("sid").join(format!("{key}.json")));
        let _ = std::fs::remove_file(self.saved.with_file_name("scrollback").join(format!("{key}.txt")));
    }

    pub(super) fn new(store: std::path::PathBuf, pipe: String, token: String) -> Registry {
        let saved = store.with_file_name("sessions.json");
        // a corrupt sessions.json is moved to sessions.json.bad, so the first persist cannot wipe it
        let mut pending = read_json::<Vec<Value>>(&saved).ok().flatten().unwrap_or_default();
        prune_dead_pending(&saved, &mut pending);
        let audit = crate::audit::Audit::new(Some(saved.with_file_name("audit.log")));
        let governor = Arc::new(crate::governor::Governor::new(crate::governor::Config::from_settings(&read_settings(&saved))));
        let pauser = pause::Pauser::new(Arc::new(crate::governor::suspend::OsSuspender), saved.with_file_name("paused.json"));
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
            run_cancels: Default::default(),
            auto_lock: Default::default(),
            flows: Default::default(),
            plist: Default::default(),
            plist_last: Default::default(),
            specs: Default::default(),
            pending: Mutex::new(pending),
            saved,
            bus: crate::msgbus::Bus::new(),
            clients: Default::default(),
            computer: AtomicBool::new(false),
            hints: Default::default(),
            modes: Default::default(),
            awaits: Default::default(),
            stall_notified: Default::default(),
            work: Default::default(),
            wt_gen: AtomicU64::new(0),
            audit,
            bad_hellos: AtomicU32::new(0),
            pulse: Default::default(),
            reapers: Default::default(),
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
            settings: Default::default(),
            hib: Default::default(),
            spawnq: Default::default(),
            governor,
            pauser,
        }
    }

    /// Throttle for token guessing: the n-th refusal in a row waits n * 50 ms (at most 1 s) before it is answered.
    /// Returns that wait; the connection sleeps it (`run_call`), so no thread of the blocking pool is held for it.
    pub(super) fn hello_failed(&self) -> Duration {
        let n = self.bad_hellos.fetch_add(1, Ordering::SeqCst).saturating_add(1);
        Duration::from_millis(u64::from(n.min(20)) * 50)
    }

    /// True when the agent in this session runs without permission prompts: whatever is typed and sent with Enter runs.
    /// The mode its own hook reported wins (it follows a Shift+Tab switch); else the start arguments, then the
    /// `defaultMode` of the agent's settings files (user level, and the worktree's `.claude/settings*.json`).
    pub(super) fn runs_in_bypass(&self, id: u32) -> bool {
        if let Some(m) = self.modes.lock_recover().get(&id) {
            return mode_is_bypass(m);
        }
        let spec = self.specs.lock_recover().get(&id).map(|s| s.to_string().to_ascii_lowercase()).unwrap_or_default();
        if ["bypasspermissions", "dangerously", "--yolo", "never-ask"].iter().any(|k| spec.contains(k)) {
            return true;
        }
        let dir = {
            let metas = self.metas.lock_recover();
            let m = metas.get(&id);
            m.and_then(|m| m["worktree"].as_str().map(String::from))
        };
        let mut files: Vec<std::path::PathBuf> = vec![];
        if let Some(h) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) {
            files.push(std::path::Path::new(&h).join(".claude").join("settings.json"));
        }
        if let Some(d) = dir {
            for f in ["settings.json", "settings.local.json"] {
                files.push(std::path::Path::new(&d).join(".claude").join(f));
            }
        }
        files
            .iter()
            .filter_map(|f| std::fs::read_to_string(f).ok())
            .filter_map(|t| serde_json::from_str::<Value>(&t).ok())
            .any(|v| settings_default_bypass(&v))
    }

    /// Types text that came from another session into an agent. No Enter follows when that agent runs in bypass mode:
    /// a hostile message would otherwise execute unreviewed.
    pub(super) fn deliver_untrusted(&self, id: u32, target: Arc<Session>, text: &str) {
        crate::msgbus::deliver(target, crate::msgbus::paste_with(text, !self.runs_in_bypass(id)));
    }

    /// True while a session waiting to be restored (from the previous daemon run) belongs to this worktree.
    pub(super) fn worktree_pending(&self, path: &str) -> bool {
        let want = norm(path);
        self.pending.lock_recover().iter().any(|r| {
            r["meta"]["worktree"].as_str().is_some_and(|w| norm(w) == want) || r["cwd"].as_str().is_some_and(|c| is_within(c, &want))
        })
    }

    /// True while a live session works in this worktree (any terminal or agent with it as worktree or cwd).
    /// What the session said it waits for: an unanswered `ask`/`escalate`, or (unless it reported with `done`) its
    /// agent's own hook event. Never the screen text.
    pub(super) fn wait_of(&self, id: u32, since_input_ms: u64, idle_ms: u64) -> Option<axelstra_core::waiting::Wait> {
        if let Some(text) = self.bus.pending_of(id) {
            return Some(axelstra_core::waiting::Wait { kind: axelstra_core::waiting::WaitKind::Question, text });
        }
        if self.bus.finished(id).is_some() {
            return None;
        }
        self.hints.waiting(id, since_input_ms, idle_ms)
    }

    /// What the session said it only waits for (CI, a deploy), while that still holds. Never after `done`.
    pub(super) fn awaiting_of(&self, id: u32, since_input_ms: u64, idle_ms: u64) -> Option<axelstra_core::waiting::AwaitView> {
        if self.bus.finished(id).is_some() {
            return None;
        }
        self.awaits.active(id, since_input_ms, idle_ms)
    }

    /// Tells the session's parent (once per wait) that it waits for an answer, with the agent's own text.
    pub(super) fn tell_parent_waiting(&self, id: u32) {
        let Some(wait) = self.hints.take_untold(id) else { return };
        let Some(pid) = self.metas.lock_recover().get(&id).and_then(|m| m["parent"].as_u64()).map(|p| p as u32) else { return };
        // a hibernated parent wakes for it: the message waits in its inbox and is typed once it is back
        if self.hib.is_sleeping(pid) {
            let m = self.bus.send(
                Some(id),
                pid,
                "message",
                &crate::prompts::waiting_report(wait.kind == axelstra_core::waiting::WaitKind::Permission, &wait.text),
                None,
                None,
            );
            let _ = m;
            if let Some(me) = self.me.get().and_then(std::sync::Weak::upgrade) {
                hibernate::wake(&me, pid);
            }
            return;
        }
        let Some(ps) = self.get(pid) else { return };
        let label = self.labels.lock_recover().get(&id).cloned().unwrap_or_else(|| crate::prompts::session_name(id));
        let text = crate::prompts::waiting_report(wait.kind == axelstra_core::waiting::WaitKind::Permission, &wait.text);
        // a `notice`, not a `message`: a message would clear the parent's own pending question and report
        let m = self.bus.send(Some(id), pid, "notice", &text, None, None);
        let agent = self.metas.lock_recover().get(&pid).map(|m| m["kind"] == "agent").unwrap_or(false);
        if agent {
            self.deliver_untrusted(pid, ps, &crate::msgbus::notice(&m, &format!("{label} ({})", crate::prompts::session_name(id))));
        }
    }

    /// Sessions that are alive and have this worktree as worktree or cwd.
    pub(super) fn live_in(&self, path: &str) -> Vec<(u32, Arc<Session>)> {
        let want = norm(path);
        // one lock at a time (the rest of the daemon takes sessions before metas)
        let alive: Vec<(u32, Arc<Session>)> =
            self.sessions.lock_recover().iter().filter(|(_, s)| s.try_wait().is_none()).map(|(id, s)| (*id, s.clone())).collect();
        let metas = self.metas.lock_recover();
        let in_meta = |id: &u32| metas.get(id).and_then(|m| m["worktree"].as_str()).map(|w| norm(w) == want).unwrap_or(false);
        let by_meta: Vec<u32> = alive.iter().map(|(id, _)| *id).filter(in_meta).collect();
        drop(metas);
        let specs = self.specs.lock_recover();
        let by_cwd = |id: &u32| specs.get(id).and_then(|sp| sp["cwd"].as_str()).map(|c| is_within(c, &want)).unwrap_or(false);
        alive.into_iter().filter(|(id, _)| by_meta.contains(id) || by_cwd(id)).collect()
    }

    /// A live session that has nothing left to do in its worktree: an agent that reported with `axelstra done` and went
    /// quiet (it only waits to be closed), or a terminal nobody typed in or saw output from for a long while and that
    /// runs nothing. Such a session does not keep its worktree from being cleaned; the janitor closes it first.
    pub(super) fn releasable(&self, id: u32, s: &Session) -> bool {
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

    /// A hibernated session belongs to this worktree: the worktree is not free to clean, the agent comes back into it.
    pub(super) fn hibernated_in(&self, path: &str) -> bool {
        let want = norm(path);
        let ids: Vec<u32> = self.hib.sleepers().into_iter().map(|(i, _)| i).collect();
        let metas = self.metas.lock_recover();
        ids.iter().any(|i| metas.get(i).and_then(|m| m["worktree"].as_str()).is_some_and(|w| norm(w) == want))
    }

    /// Notes that a session left this worktree just now.
    pub(super) fn note_activity(&self, path: &str) {
        self.activity.lock_recover().insert(norm(path), Instant::now());
    }

    /// For a worktree the user made by hand: a session of any kind is open there, or one was a moment ago (or this
    /// daemon is too new to know).
    pub(super) fn worktree_touched(&self, path: &str) -> bool {
        let quiet = self.timing.quiet;
        if self.started.elapsed() < quiet || !self.live_in(path).is_empty() || self.hibernated_in(path) {
            return true;
        }
        self.activity.lock_recover().get(&norm(path)).is_some_and(|at| at.elapsed() < quiet)
    }

    /// When the janitor must look again without any event (see `janitor_wake`). Forgets activity notes that no longer
    /// matter.
    pub(super) fn next_janitor_wake(&self) -> Option<Duration> {
        let mut activity = self.activity.lock_recover();
        activity.retain(|_, at| at.elapsed() < self.timing.quiet);
        janitor_wake(&self.timing, self.started.elapsed(), activity.values().map(Instant::elapsed), crate::janitor::awaiting_remote())
    }

    /// Closes every session of this worktree (agents, shells, whatever they started: the whole process tree dies) and
    /// waits until they are gone, so the folder is no longer anyone's cwd. Returns their labels.
    pub(super) fn close_worktree(&self, path: &str) -> Vec<String> {
        let doomed = self.live_in(path);
        if doomed.is_empty() {
            return vec![];
        }
        let sessions: Vec<Arc<Session>> = doomed.iter().map(|(_, s)| s.clone()).collect();
        self.tear_down(&sessions);
        let mut labels = vec![];
        for (id, _) in &doomed {
            labels.push(self.labels.lock_recover().get(id).cloned().unwrap_or_else(|| unnamed_label(*id)));
            self.forget_session(*id);
        }
        self.note_activity(path);
        self.persist();
        self.pulse.bump();
        labels
    }

    /// Kills the sessions' process trees and waits until they are gone, then kills whatever of the trees outlived the
    /// kill (a process that broke out of its job), and counts sessions, processes and RAM released.
    pub(super) fn tear_down(&self, sessions: &[Arc<Session>]) {
        let snap = axelstra_core::procs::snapshot();
        let ids: Vec<u32> =
            self.sessions.lock_recover().iter().filter(|(_, x)| sessions.iter().any(|s| Arc::ptr_eq(s, x))).map(|(id, _)| *id).collect();
        for id in ids {
            self.resume_before_kill(id);
        }
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
    pub(super) fn forget_session(&self, id: u32) -> bool {
        let was = self.sessions.lock_recover().remove(&id).is_some();
        self.labels.lock_recover().remove(&id);
        self.metas.lock_recover().remove(&id);
        let gone = self.specs.lock_recover().remove(&id);
        if let Some(spec) = gone {
            self.drop_sid_of(&spec);
        }
        self.bus.forget(id);
        self.hib.forget(id);
        was
    }

    /// A session whose program ended on its own leaves the registry, so `session.list`, `axelstra ls` and
    /// `sessions.json` show only what runs. Tells the watchers `session.exited {id, code}` (the app keeps a failed
    /// terminal's tab until the user closes it, and shows the code) and `dispatch.changed`, and wakes the session
    /// monitor and the janitor. Its exit code stays answerable through `session.status` (`exited`). Only the one that
    /// really removed it announces, so a kill racing the exit does not announce twice. Whatever it reported with
    /// `axelstra done` is already in its parent's inbox and its task file (written when `done` was called).
    pub(super) fn retire(&self, id: u32, code: Option<i32>) -> bool {
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
    pub(super) fn exited_of(&self, id: u32) -> Option<(Option<i32>, bool)> {
        self.exited.lock_recover().iter().rev().find(|e| e.id == id).map(|e| (e.code, e.finished))
    }

    pub(super) fn worktree_busy(&self, path: &str) -> bool {
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
        in_flow || self.orch_active(path) || self.hibernated_in(path) || self.live_in(path).iter().any(|(id, s)| !self.releasable(*id, s))
    }

    /// True while an orchestrator run of this project is going and `path` is one of its worktrees (`<name>-integ`,
    /// `<name>-<card>` in `.axelstra-wt`): the run owns them until it ends, whatever the janitor thinks of their branches.
    pub(super) fn orch_active(&self, path: &str) -> bool {
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
    pub(super) fn worktree_working(&self, path: &str) -> bool {
        let want = norm(path);
        let working: Vec<u32> =
            self.sessions.lock_recover().iter().filter(|(_, s)| s.try_wait().is_none() && s.idle_ms() < 5000).map(|(id, _)| *id).collect();
        let metas = self.metas.lock_recover();
        working.iter().any(|id| metas.get(id).and_then(|m| m["worktree"].as_str()).map(|w| norm(w) == want).unwrap_or(false))
    }

    /// The running session that carries conversation key `key` (`meta.key`), if any.
    pub(super) fn live_by_key(&self, key: &str) -> Option<u32> {
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

    /// The project's main agent: the live agent session marked `main` for that project id (the lowest id if several).
    pub(super) fn project_main(&self, project: &str) -> Option<u32> {
        let sessions = self.sessions.lock_recover();
        let metas = self.metas.lock_recover();
        project_main_in(&sessions, &metas, project)
    }

    pub(super) fn get(&self, id: u32) -> Option<Arc<Session>> {
        self.sessions.lock_recover().get(&id).cloned()
    }

    /// Starts a session and records its label and UI metadata.
    pub(super) fn spawn(self: &Arc<Self>, s: &Spawn, label: Option<&str>, meta: Option<Value>) -> Result<u32> {
        let id = self.next.fetch_add(1, Ordering::SeqCst) + 1;
        self.spawn_as(id, s, label, meta)
    }

    /// `spawn` under an id the caller chose: a hibernated session wakes under the id it had.
    pub(super) fn spawn_as(self: &Arc<Self>, id: u32, s: &Spawn, label: Option<&str>, meta: Option<Value>) -> Result<u32> {
        // the session learns its own id, so a CLI run inside it can name it as the parent of a worker
        self.agent_tokens.lock_recover().retain(|_, v| *v != id);
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
        // build cost: a derived job count, optional sccache and shared caches, computed now from the free memory
        // (no timer); a variable the user or the spawn request already sets is left alone
        if let Some(data) = self.saved.parent() {
            let settings = self.with_settings(crate::buildenv::BuildSettings::from_settings);
            let project = meta.as_ref().and_then(|m| m["project"].as_str());
            let facts = crate::buildenv::facts_now(s.cwd.as_deref(), data);
            let given = s.env.clone();
            let extra = crate::buildenv::build_env(&settings, project, &facts, |k| {
                std::env::var_os(k).is_some() || given.iter().any(|(g, _)| g.eq_ignore_ascii_case(k))
            });
            s.env.extend(extra);
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
pub(super) fn watch_exit(reg: &Arc<Registry>, id: u32, s: Arc<Session>) {
    let reg = Arc::downgrade(reg);
    std::thread::spawn(move || {
        s.wait_exit();
        s.kill();
        // the last output may still be on its way through the pipe: then the stream ends, which is what tells a
        // terminal's subscribers that the program is gone
        std::thread::sleep(Duration::from_millis(150));
        s.end_stream();
        let Some(reg) = reg.upgrade() else { return };
        // put to sleep (or already woken under the same id): this process ending is not the session ending
        if hibernate::superseded(&reg, id, &s) {
            return;
        }
        if let Some(w) = reg.metas.lock_recover().get(&id).and_then(|m| m["worktree"].as_str().map(String::from)) {
            reg.note_activity(&w);
        }
        reg.hints.clear(id);
        reg.modes.lock_recover().remove(&id);
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

/// `Registry::project_main` for a caller that already holds the `sessions` and `metas` locks.
pub(super) fn project_main_in(sessions: &HashMap<u32, Arc<Session>>, metas: &HashMap<u32, Value>, project: &str) -> Option<u32> {
    metas
        .iter()
        .filter(|(id, m)| {
            m["kind"] == "agent"
                && m["main"] == true
                && m["project"].as_str() == Some(project)
                && sessions.get(id).is_some_and(|s| s.try_wait().is_none())
        })
        .map(|(id, _)| *id)
        .min()
}

pub(super) fn new_agent_token() -> Option<String> {
    let mut b = [0u8; 24];
    getrandom::getrandom(&mut b).ok()?;
    Some(b.iter().map(|x| format!("{x:02x}")).collect())
}

/// False when a saved session points at folders that are all gone (a deleted worktree): it cannot be restored.
/// A record with no folder at all (a plain shell) counts as alive.
pub(super) fn rec_alive(rec: &Value) -> bool {
    let dirs: Vec<&str> = [rec["meta"]["worktree"].as_str(), rec["cwd"].as_str()].into_iter().flatten().filter(|d| !d.is_empty()).collect();
    dirs.is_empty() || dirs.iter().any(|d| std::path::Path::new(d).exists())
}

/// One-off clean-up at start: sessions saved for worktrees that no longer exist are dropped, so a restart does not
/// try to bring back dozens of dead agents. The file as it was is copied to `backups/` first.
pub(super) fn prune_dead_pending(saved: &std::path::Path, pending: &mut Vec<Value>) {
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
pub(super) fn rec_junk(rec: &Value) -> bool {
    let has = |v: &Value| v.as_str().is_some_and(|s| !s.is_empty());
    let meta = &rec["meta"];
    !has(&rec["cwd"]) && !["worktree", "project", "key", "kind"].iter().any(|k| has(&meta[*k]))
}

/// Under test, the real data dir's files are never written (the same rule `serve_with_store` applies to the daemon).
pub(super) fn real_dir_under_test(saved: &std::path::Path) -> bool {
    saved.parent().is_some_and(|d| crate::proto::refuse_real_when_testing("", d).is_err())
}

/// Ephemeral worker: when it finishes (process exit, or quiet for `idle_secs` in "idle" mode) its last
/// output goes to the parent session as if typed, and the worker is closed so it stops using memory.
/// Typical use: a test run or a helper agent that reports back to the main agent and disappears.
/// How long a session that already called `axelstra done` must be quiet before it stops counting as busy.
pub(super) const REPORTED_IDLE_MS: u64 = 10_000;

/// How long a terminal (not an agent) must have been quiet, with nothing running in it, before it no longer keeps its
/// worktree from being cleaned up.
pub(super) const SHELL_IDLE_MS: u64 = 10 * 60_000;

/// Kills the sessions (whole process trees) and waits, up to `limit`, until each one is gone.
pub(super) fn kill_and_wait(sessions: &[Arc<Session>], limit: Duration) {
    for s in sessions {
        s.kill();
    }
    let end = std::time::Instant::now() + limit;
    while std::time::Instant::now() < end && sessions.iter().any(|s| s.try_wait().is_none()) {
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[cfg(test)]
mod close_label_tests {
    use super::*;

    #[test]
    fn a_closed_session_without_a_label_is_named_without_any_language() {
        assert_eq!(unnamed_label(7), "#7");
        let dir = std::env::temp_dir().join(format!("axl-close-label-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let reg = Arc::new(Registry::new(dir.join("projects.json"), "p".into(), "t".into()));
        let (prog, args): (&str, Vec<String>) =
            if cfg!(windows) { ("cmd", vec!["/c".into(), "ping -n 30 127.0.0.1 >nul".into()]) } else { ("sleep", vec!["30".into()]) };
        let spawn = Spawn { program: prog.into(), args, cwd: Some(dir.clone()), env: vec![], cols: 80, rows: 24 };
        let id = reg.spawn(&spawn, None, Some(json!({"worktree": dir.to_string_lossy()}))).unwrap();
        reg.labels.lock_recover().remove(&id);
        let labels = reg.close_worktree(&dir.to_string_lossy());
        assert_eq!(labels, vec![format!("#{id}")]);
    }
}
