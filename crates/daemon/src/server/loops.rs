//! The background loops of the daemon: reapers, the scheduler, the janitor, the session monitor, the stall watchdog.
//! Each sleeps until an event wakes it (see `Pulse`); the timers are the moments something becomes due.

use super::*;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub(super) static SWEEP_QUEUED: AtomicBool = AtomicBool::new(false);
/// A backoff retry of a pending removal is already waiting.
pub(super) static RETRY_QUEUED: AtomicBool = AtomicBool::new(false);

/// Kicks the janitor a few seconds from now (once for a burst of events): after a `done` or a merge, so the finished
/// worktree goes away without waiting for anything else. The delay lets the reaper close the session; it is one
/// delay per event, not a loop.
pub(super) fn sweep_soon(reg: &Arc<Registry>) {
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
pub(super) fn tell_main_not_integrated(reg: &Arc<Registry>, item: &Value) {
    let Some(project) = item["project"].as_str() else { return };
    let Some((id, s)) = reg.project_main(project).and_then(|id| reg.get(id).map(|s| (id, s))) else { return };
    let text = crate::prompts::not_integrated(
        item["path"].as_str().unwrap_or_default(),
        item["branch"].as_str().unwrap_or_default(),
        item["why"].as_str().unwrap_or_default(),
    );
    let m = reg.bus.send(None, id, "message", &text, None, None);
    reg.deliver_untrusted(id, s, &crate::msgbus::notice(&m, "axelstra"));
}

/// Everything that closes a session by itself when it has been quiet: the reapers of ephemeral workers and leads, and
/// the done-closers of agents left running after `axelstra done`. ONE thread serves all of them (not one per worker):
/// it evaluates every task when the pulse moves and otherwise sleeps until the earliest deadline among them (the moment
/// a session would count as idle long enough, or the end of a drain). With no task the thread ends; the next task
/// starts it again. No fixed sleep anywhere.
#[derive(Default)]
pub(super) struct Reapers {
    state: Mutex<ReaperState>,
    /// scheduler threads started so far (tests: many tasks still make one)
    pub(super) threads_started: AtomicU64,
}

#[derive(Default)]
struct ReaperState {
    tasks: Vec<Task>,
    running: bool,
}

impl Reapers {
    /// Tasks waiting right now (one being evaluated counts as none).
    #[cfg(test)]
    pub(super) fn pending(&self) -> usize {
        self.state.lock_recover().tasks.len()
    }
}

/// How long the output pipe gets to drain after a worker's process ended, before its screen is read.
const DRAIN: Duration = Duration::from_millis(300);

enum Task {
    Reaper(ReaperTask),
    Closer(CloserTask),
}

struct ReaperTask {
    id: u32,
    parent: Option<u32>,
    idle_mode: bool,
    idle_secs: u64,
    lines: usize,
    worked: bool,
    done_by_idle: bool,
    nudged: bool,
    /// set once it decided to end: the report goes out when this moment came
    drain_until: Option<Instant>,
}

struct CloserTask {
    id: u32,
    armed: Instant,
}

/// What a task wants after a look at its session.
enum Step {
    /// look again at the next pulse, or at this moment
    Wait(Option<Instant>),
    /// nothing left to do
    Drop,
    /// the report is due now
    Finish,
    /// the done-closer's quiet spell is over
    Close,
}

impl ReaperTask {
    fn new(id: u32, parent: Option<u32>, idle_mode: bool, idle_secs: u64, lines: usize) -> Self {
        ReaperTask { id, parent, idle_mode, idle_secs, lines, worked: false, done_by_idle: false, nudged: false, drain_until: None }
    }

    fn step(&mut self, reg: &Arc<Registry>, now: Instant) -> Step {
        if let Some(t) = self.drain_until {
            return if now >= t { Step::Finish } else { Step::Wait(Some(t)) };
        }
        let Some(s) = reg.get(self.id) else { return Step::Drop };
        let exited = s.try_wait();
        // frozen by the memory governor (or its builds): the silence is ours, it is not finished. Resuming wakes the pulse.
        if exited.is_none() && reg.pauser.paused_of(self.id).is_some() {
            return Step::Wait(None);
        }
        let idle = s.idle_ms();
        if idle < 3000 {
            self.worked = true;
        }
        let mut end = exited.is_some() || reg.bus.finished(self.id).is_some();
        if !end && self.idle_mode && self.worked && idle >= self.idle_secs * 1000 {
            // it said it waits for an answer (hook event, ask, escalate): not finished, keep it. The parent was told
            // when the event came in.
            // or that it only waits for CI or a deploy (`axelstra await`): quiet on purpose
            if reg.wait_of(self.id, s.since_input_ms(), idle).is_some() || reg.awaiting_of(self.id, s.since_input_ms(), idle).is_some() {
                self.worked = false;
            } else if !self.nudged && reg.metas.lock_recover().get(&self.id).map(|m| m["kind"] == "agent").unwrap_or(false) {
                // first quiet spell: ask the worker to report through the bus instead of scraping its screen
                crate::msgbus::deliver(s.clone(), crate::msgbus::paste(crate::prompts::IDLE_NUDGE));
                self.nudged = true;
                self.worked = false;
            } else {
                self.done_by_idle = true;
                end = true;
            }
        }
        if end {
            // give the output pipe a moment to drain after the process ended
            self.drain_until = Some(now + DRAIN);
            return Step::Wait(self.drain_until);
        }
        // the one timer: the moment the session would count as idle for `idle_secs`
        let limit = self.idle_secs * 1000;
        Step::Wait((self.idle_mode && self.worked && idle < limit).then(|| now + Duration::from_millis(limit - idle + 20)))
    }

    /// Reports to the parent and removes the session.
    fn finish(self, reg: &Arc<Registry>) {
        let id = self.id;
        // a hibernated parent comes back (and types what reached it) before the report is pasted into its terminal
        if let Some(p) = self.parent {
            hibernate::wake_wait(reg, p, true, Duration::from_secs(60));
        }
        let Some(s) = reg.get(id) else { return };
        let code = s.try_wait();
        let done_msg = reg.bus.finished(id);
        // what the terminal shows, not the escape-code soup of a TUI redrawing itself
        // for an agent TUI only its last reply, not the banner, tool-run lines and status bar around it
        let tail = axelstra_core::screen::last_reply(&s.screen_lines(200)).unwrap_or_else(|| s.screen_lines(self.lines).join("\n"));
        let label = reg.labels.lock_recover().get(&id).cloned().unwrap_or_else(|| crate::prompts::session_name(id));
        let status = crate::prompts::worker_status(done_msg.is_some(), self.done_by_idle, code);
        if let Some(parent) = self.parent.and_then(|p| reg.get(p).map(|s| (p, s))) {
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
        reg.resume_before_kill(id);
        s.kill();
        // the report went to the parent above; now the session leaves the registry (and the watchers hear it)
        reg.retire(id, code);
    }
}

impl CloserTask {
    fn step(&self, reg: &Arc<Registry>, now: Instant) -> Step {
        let Some(s) = reg.get(self.id) else { return Step::Drop };
        if s.try_wait().is_some() || reg.bus.finished(self.id).is_none() {
            return Step::Drop;
        }
        // the user typed after the report: the session is theirs again
        if s.since_input_ms() < self.armed.elapsed().as_millis() as u64 {
            return Step::Drop;
        }
        let idle = s.idle_ms();
        if idle >= REPORTED_IDLE_MS {
            return Step::Close;
        }
        Step::Wait(Some(now + Duration::from_millis(REPORTED_IDLE_MS - idle + 20)))
    }

    /// Closes the session unless a question was asked after the report. Runs on its own short-lived thread: ending the
    /// process tree can take seconds and must not hold up the other tasks.
    fn close(self, reg: Arc<Registry>) {
        let id = self.id;
        std::thread::spawn(move || {
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
}

/// An agent that reported with `axelstra done` and then stays quiet for `REPORTED_IDLE_MS` is closed by itself (leads and
/// plain workers too; ephemeral ones have `spawn_reaper`): its process tree ends and its tab and record go. One the user
/// typed into after the report is theirs again and stays open. A session opened by hand that never said `done` is never
/// closed here.
pub(super) fn spawn_done_closer(reg: Arc<Registry>, id: u32) {
    add_task(&reg, Task::Closer(CloserTask { id, armed: Instant::now() }));
}

/// The shortest quiet spell after which an ephemeral worker may be nudged and then closed. An agent in a long tool call,
/// a slow network or a build that prints nothing is silent for a minute without being done; a plain command is not.
pub(crate) fn reaper_min_idle_secs(agent: bool) -> u64 {
    if agent {
        120
    } else {
        3
    }
}

pub(super) fn spawn_reaper(reg: Arc<Registry>, id: u32, parent: Option<u32>, idle_mode: bool, idle_secs: u64, lines: usize) {
    add_task(&reg, Task::Reaper(ReaperTask::new(id, parent, idle_mode, idle_secs, lines)));
}

/// Queues a task and makes sure the scheduler thread runs.
fn add_task(reg: &Arc<Registry>, task: Task) {
    {
        let mut st = reg.reapers.state.lock_recover();
        st.tasks.push(task);
        if !st.running {
            st.running = true;
            reg.reapers.threads_started.fetch_add(1, Ordering::SeqCst);
            let r = reg.clone();
            std::thread::spawn(move || {
                let _site = axelstra_core::proc::site("reapers");
                run_reapers(r)
            });
        }
    }
    // a thread that is already running looks at the new task now
    reg.pulse.bump();
}

/// The scheduler thread: evaluates every task when the pulse moves, otherwise sleeps until the earliest deadline.
fn run_reapers(reg: Arc<Registry>) {
    let mut seen = reg.pulse.now();
    loop {
        let batch = std::mem::take(&mut reg.reapers.state.lock_recover().tasks);
        let now = Instant::now();
        let mut keep = Vec::with_capacity(batch.len());
        let mut deadline: Option<Instant> = None;
        for mut task in batch {
            let step = match &mut task {
                Task::Reaper(t) => t.step(&reg, now),
                Task::Closer(t) => t.step(&reg, now),
            };
            match (step, task) {
                (Step::Wait(at), task) => {
                    deadline = match (deadline, at) {
                        (Some(a), Some(b)) => Some(a.min(b)),
                        (a, b) => a.or(b),
                    };
                    keep.push(task);
                }
                (Step::Finish, Task::Reaper(t)) => t.finish(&reg),
                (Step::Close, Task::Closer(t)) => t.close(reg.clone()),
                _ => {}
            }
        }
        {
            let mut st = reg.reapers.state.lock_recover();
            st.tasks.extend(keep);
            if st.tasks.is_empty() {
                st.running = false;
                return;
            }
        }
        seen = reg.pulse.wait(seen, deadline.map(|d| d.saturating_duration_since(Instant::now())));
    }
}

/// Kills the live sessions whose worktree folder no longer exists on disk (they would only keep using memory).
/// Sessions without a worktree are left alone. No timer: called when something may have removed a worktree (the
/// project list is asked for, the watched worktree vanished, a janitor sweep or an orchestration ended).
pub(super) fn reap_orphans(reg: &Registry) {
    kill_sessions_where(reg, |w| !std::path::Path::new(w).exists());
}

/// Kills the live sessions whose worktree (from their meta) satisfies `pred`.
pub(super) fn kill_sessions_where(reg: &Registry, pred: impl Fn(&str) -> bool) {
    let doomed: Vec<_> = {
        let sessions = reg.sessions.lock_recover();
        let metas = reg.metas.lock_recover();
        sessions
            .iter()
            .filter(|(_, s)| s.try_wait().is_none())
            .filter(|(id, _)| metas.get(id).and_then(|m| m["worktree"].as_str()).is_some_and(&pred))
            .map(|(id, s)| (*id, s.clone()))
            .collect()
    };
    for (id, s) in doomed {
        reg.resume_before_kill(id);
        s.kill();
    }
}

/// Starts the automations when they are due. No tick: it sleeps until the next one is due, or until the list changes
/// (`auto_wake`). Missed runs are not replayed.
pub(super) fn spawn_scheduler(reg: Arc<Registry>) {
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
                if !claimed.is_empty() {
                    // their last run moved
                    r.broadcast("automations.changed");
                }
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

/// Reads the spending and reports level crossings. It sleeps until something that can change the spending happens
/// (`budget_wake`): the settings changed, the usage watcher saw an agent's log grow (`spawn_usage_watch`), or a session
/// changed state. A burst of wake-ups is one reading (the permit of `Notify` does not queue, and the short pause
/// lets the burst finish first).
pub(super) fn spawn_budget_loop(reg: Arc<Registry>) {
    spawn_usage_watch(reg.clone());
    tokio::spawn(async move {
        loop {
            let r = reg.clone();
            let _ = tokio::task::spawn_blocking(move || misc::budget_check(&r)).await;
            reg.budget_wake.notified().await;
            tokio::time::sleep(BUDGET_COALESCE).await;
        }
    });
}

/// Pause after a wake-up of the budget loop, so the writes of one agent turn are read once.
const BUDGET_COALESCE: Duration = Duration::from_millis(500);
/// A usage burst is announced after this much quiet, or at the latest after `USAGE_MAX_WAIT` of constant writing.
const USAGE_QUIET: Duration = Duration::from_millis(400);
const USAGE_MAX_WAIT: Duration = Duration::from_secs(3);

/// What an agent's files say changed: `usage` (tokens spent) and/or `limits` (rate limits, context).
#[derive(Clone, Copy, Default, PartialEq, Debug)]
pub(super) struct UsageKinds {
    pub usage: bool,
    pub limits: bool,
}

impl UsageKinds {
    fn merge(&mut self, o: UsageKinds) {
        self.usage |= o.usage;
        self.limits |= o.limits;
    }
}

/// A folder the usage watcher looks at, what a change under it means, and whether only `*.json` files count.
pub(super) struct UsageRoot {
    pub dir: PathBuf,
    pub kinds: UsageKinds,
    pub json_only: bool,
}

/// The folders of the agents' usage files: Claude's transcripts (tokens), Codex's session files (tokens and its rate
/// limits), Gemini's chats (tokens) and the status line files `axelstra statusline-tee` saves in `status/` (Claude's
/// context and 5 h / weekly limits).
pub(super) fn usage_roots(data_dir: &Path) -> Vec<UsageRoot> {
    let usage = UsageKinds { usage: true, limits: false };
    let mut v = vec![UsageRoot { dir: data_dir.join("status"), kinds: UsageKinds { usage: false, limits: true }, json_only: true }];
    if let Some(d) = crate::agentlog::claude_root() {
        v.push(UsageRoot { dir: d, kinds: usage, json_only: false });
    }
    if let Some(d) = crate::agentlog::codex_root() {
        v.push(UsageRoot { dir: d, kinds: UsageKinds { usage: true, limits: true }, json_only: false });
    }
    if let Some(d) = crate::agentlog::gemini_root() {
        v.push(UsageRoot { dir: d, kinds: usage, json_only: false });
    }
    v
}

/// What a file system event means for `roots` (none for a read, or a path outside them).
///
/// `canon[i]` is the resolved form of `roots[i].dir`, computed once (see `resolve_root`) instead of per event.
fn usage_kinds_of(ev: &notify::Event, roots: &[UsageRoot], canon: &[Option<PathBuf>]) -> UsageKinds {
    let mut k = UsageKinds::default();
    if matches!(ev.kind, notify::EventKind::Access(_)) {
        return k;
    }
    for p in &ev.paths {
        for (r, c) in roots.iter().zip(canon) {
            if !r.json_only || p.extension().is_some_and(|x| x == "json") {
                // FSEvents (macOS) reports canonical paths (`/private/var/...` for `/var/...`), so a root spelled
                // through a symlink only matches by its resolved form
                if p.starts_with(&r.dir) || c.as_ref().is_some_and(|c| p.starts_with(c)) {
                    k.merge(r.kinds);
                }
            }
        }
    }
    k
}

/// The resolved form of each root's folder (`None` while it does not exist). Called when the watcher starts and again
/// for a root only when it first appears, so an event never touches the file system to compare paths.
fn resolve_root(r: &UsageRoot) -> Option<PathBuf> {
    r.dir.canonicalize().ok()
}

/// Watches `roots` and calls `emit` once per burst of writes, with what changed. Asleep until the OS reports a write. A
/// folder that does not exist yet (the user never ran that agent) is watched through its nearest existing parent and
/// picked up when it appears. Runs on its own thread until the process ends.
pub(super) fn watch_usage(roots: Vec<UsageRoot>, emit: impl Fn(UsageKinds) + Send + 'static) {
    use notify::{RecursiveMode, Watcher};
    std::thread::spawn(move || {
        let (tx, rx) = std::sync::mpsc::channel();
        let Ok(mut w) = notify::recommended_watcher(tx) else { return };
        let mut pending: Vec<usize> = vec![];
        let canon: std::cell::RefCell<Vec<Option<PathBuf>>> = std::cell::RefCell::new(roots.iter().map(resolve_root).collect());
        let try_watch = |w: &mut notify::RecommendedWatcher, pending: &mut Vec<usize>| -> UsageKinds {
            let mut came = UsageKinds::default();
            pending.retain(|&i| {
                if roots[i].dir.is_dir() && w.watch(&roots[i].dir, RecursiveMode::Recursive).is_ok() {
                    canon.borrow_mut()[i] = resolve_root(&roots[i]);
                    came.merge(roots[i].kinds);
                    return false;
                }
                true
            });
            came
        };
        let mut parents: HashSet<PathBuf> = HashSet::new();
        // the nearest existing folder above each missing root: it tells when the next level appears
        let arm_parents = |w: &mut notify::RecommendedWatcher, pending: &[usize], parents: &mut HashSet<PathBuf>| {
            for &i in pending {
                if let Some(up) = roots[i].dir.ancestors().skip(1).find(|a| a.is_dir()) {
                    if parents.insert(up.to_path_buf()) {
                        let _ = w.watch(up, RecursiveMode::NonRecursive);
                    }
                }
            }
        };
        pending.extend(0..roots.len());
        let _ = try_watch(&mut w, &mut pending);
        arm_parents(&mut w, &pending, &mut parents);
        while let Ok(first) = rx.recv() {
            let mut kinds = UsageKinds::default();
            let mut take = |ev: notify::Result<notify::Event>, w: &mut notify::RecommendedWatcher, pending: &mut Vec<usize>| {
                let Ok(ev) = ev else { return };
                kinds.merge(usage_kinds_of(&ev, &roots, &canon.borrow()));
                // a watched folder's parent saw something: the folder may exist now
                if !pending.is_empty() && !matches!(ev.kind, notify::EventKind::Access(_)) {
                    kinds.merge(try_watch(w, pending));
                    arm_parents(w, pending, &mut parents);
                }
            };
            take(first, &mut w, &mut pending);
            // one burst, one announcement: wait for quiet, but never past the cap while an agent keeps writing
            let start = Instant::now();
            while let Some(left) = USAGE_MAX_WAIT.checked_sub(start.elapsed()) {
                match rx.recv_timeout(USAGE_QUIET.min(left)) {
                    Ok(ev) => take(ev, &mut w, &mut pending),
                    Err(_) => break,
                }
            }
            if kinds != UsageKinds::default() {
                emit(kinds);
            }
        }
    });
}

/// Starts the usage watcher: `usage.changed` and `limits.changed` on the watch stream, and a wake-up of the budget loop.
fn spawn_usage_watch(reg: Arc<Registry>) {
    let Some(dir) = reg.saved.parent().map(Path::to_path_buf) else { return };
    watch_usage(usage_roots(&dir), move |k| {
        if k.usage {
            reg.broadcast("usage.changed");
            reg.budget_wake.notify_one();
        }
        if k.limits {
            reg.broadcast("limits.changed");
        }
    });
}

/// Tells the app when a file in the data directory that other processes write changes: `dispatch.json` is written by
/// the `axelstra dispatch` commands, which do not go through the daemon. Asleep until the OS reports a write.
pub(super) fn spawn_data_watch(reg: Arc<Registry>) {
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
/// and, only while a worktree with its own commits waits to be integrated, one remote deadline (30 min) for what raises
/// no local event, like a PR merged on GitHub (a fetch that brings the merge, and the PR watcher seeing a PR merged,
/// wake it by event: `prwatch`). `janitor.off` turns it off.
/// Wakes the janitor again after the backoff delay of attempt `attempts` (one waiting timer at a time).
pub(super) fn queue_janitor_retry(reg: &Arc<Registry>, attempts: u32) {
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

pub(super) fn spawn_janitor(reg: Arc<Registry>) {
    let gap = reg.timing.gap;
    std::thread::spawn(move || {
        let _site = axelstra_core::proc::site("janitor");
        let mut seen = 0;
        let mut last: Option<Instant> = None;
        let mut attempts = 0u32;
        loop {
            // events wake it; the timer is the next moment something becomes removable by time alone, or (only while a
            // worktree waits for its PR to be merged on GitHub) the one remote deadline; with neither, no timer
            seen = reg.janitor.wait(seen, reg.next_janitor_wake());
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
            // the build artifacts of worktrees nobody used for `janitor.artifact_quiet_hours` (`server/disk.rs`); it also
            // sets the next artifact deadline that `next_janitor_wake` sleeps until
            if reg.with_settings(crate::disk::Config::from_settings).enabled {
                let art = super::disk::pass(&reg, None, false, false, true);
                if art["freed_mb"].as_u64().is_some_and(|n| n > 0) {
                    eprintln!("janitor: build artifacts freed {} MB", art["freed_mb"]);
                }
            } else {
                *reg.disk.due.lock_recover() = None;
            }
            // a low volume left after both: reclaim further down the list (same rules), or say so
            super::disk::ensure(&reg, true);
            reg.announce_events();
        }
    });
}

/// Tells watching connections (`session.changed`, see `ensure_session_notifier`) when the state of any session
/// changed: started, exited, went quiet or working again, escalated, got a message... Woken by those events (`pulse`);
/// its only timer is the moment the earliest working session would count as quiet, and with every session quiet it
/// sleeps until the next event. An agent that just went quiet or ended has its screen saved, so a daemon that is
/// killed or crashes still leaves the restored tab its history.
pub(super) fn spawn_session_monitor(reg: Arc<Registry>) {
    use axelstra_core::pty::QUIET_MS;
    std::thread::spawn(move || {
        let _site = axelstra_core::proc::site("session_monitor");
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
                // sessions started, ended or went quiet/working: what they cost in memory and tokens moved with them
                reg.broadcast_resources();
                reg.budget_wake.notify_one();
                save_histories_of(&reg, &settled);
            }
            seen = reg.pulse.wait(seen, next.map(|ms| Duration::from_millis(ms + 20)));
        }
    });
}

/// Fresh worktree + agent session with the automation's prompt. Records the start first so a failure never loops.
/// Marks the automation as started now and returns it. The caller holds `auto_lock`.
pub(super) fn claim_automation(path: &std::path::Path, id: &str) -> Result<crate::automations::Automation> {
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

pub(super) fn run_automation(reg: &Arc<Registry>, path: &std::path::Path, id: &str) -> Result<Value> {
    let a = {
        let _g = reg.auto_lock.lock_recover();
        claim_automation(path, id)?
    };
    launch_automation(reg, &a)
}

/// Fresh worktree + agent session for a claimed automation. Takes no lock of its own.
pub(super) fn launch_automation(reg: &Arc<Registry>, a: &crate::automations::Automation) -> Result<Value> {
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

/// Keeps a stray daemon from lingering: with no connection, no running session and no enabled automation for
/// ten minutes (`AXELSTRA_IDLE_EXIT_SECS`, 0 turns it off) it exits. The app normally stops it on quit.
/// No polling: it looks again only when something happened (`pulse`: a connection, a session exiting, a request),
/// and the one timer is the exit deadline, armed while idle.
pub(super) fn spawn_idle_exit(reg: Arc<Registry>) {
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

pub(super) type Picked = (u32, Arc<Session>, axelstra_core::pty::View, Option<String>, Option<Value>);

/// Parents of the sessions that still run for them: a parent waiting on live children is not stalled.
pub(super) fn busy_parents(reg: &Registry, picked: &[Picked]) -> std::collections::HashSet<u32> {
    picked
        .iter()
        .filter(|(id, s, ..)| s.try_wait().is_none() && reg.bus.finished(*id).is_none())
        .filter_map(|(_, _, _, _, m)| m.as_ref().and_then(|m| m["parent"].as_u64()).map(|p| p as u32))
        .collect()
}

/// What the stall watchdog needs to know about a session (see `stall::is_stalled`).
pub(super) fn stall_facts(
    reg: &Registry,
    id: u32,
    running: bool,
    idle_ms: u64,
    since_input_ms: u64,
    meta: Option<&Value>,
    busy: &std::collections::HashSet<u32>,
) -> crate::stall::Facts {
    let parent = meta.and_then(|m| m["parent"].as_u64()).map(|p| p as u32);
    // a parent that is a live agent session handles a stall of its child; anything else leaves it to the user
    let parent_idle_ms = parent.and_then(|p| {
        let agent = reg.metas.lock_recover().get(&p).is_some_and(|m| m["kind"] == "agent");
        let s = reg.get(p)?;
        (agent && s.try_wait().is_none()).then(|| s.idle_ms())
    });
    let full = || {
        let alive = |i: u32| reg.get(i).is_some_and(|s| s.try_wait().is_none());
        reg.bus.live_workers(alive) >= reg.bus.limit()
    };
    let since_msg_ms = reg.bus.since_msg_ms(id);
    crate::stall::Facts {
        agent: meta.is_some_and(|m| m["kind"] == "agent"),
        main: meta.is_some_and(|m| m["main"] == true),
        running,
        reported: reg.bus.finished(id).is_some(),
        escalated: reg.bus.is_escalated(id),
        awaiting: reg.awaiting_of(id, since_input_ms, idle_ms).is_some(),
        live_children: busy.contains(&id),
        paused: reg.pauser.paused_of(id).is_some(),
        idle_ms,
        since_input_ms,
        since_msg_ms,
        bg_work: reg.work.busy(id),
        slot_wait: reg.work.slot_waiting(id, full),
        parent_idle_ms,
        unasked: crate::stall::never_asked(meta.is_some_and(|m| m["unrequested"] == true), since_input_ms, since_msg_ms),
        notified_ms: reg.stall_notified.lock_recover().get(&id).map(|t| t.elapsed().as_millis() as u64),
    }
}

/// An agent that became stalled gets one message to its parent per episode (until it works again); the user hears of
/// it (`session.list` `stalled`) only when nobody handles it. No tick: it looks when the pulse moves (a session
/// changed, a message, a hint, the settings) and at the one computed moment the next decision falls due
/// (`stall::next_check_ms`, `stall::next_human_ms`).
pub(super) fn spawn_stall_watchdog(reg: Arc<Registry>) {
    std::thread::spawn(move || {
        let _site = axelstra_core::proc::site("stall_watchdog");
        let mut seen = reg.pulse.now();
        loop {
            let next = stall_tick(&reg);
            seen = reg.pulse.wait(seen, next);
        }
    });
}

/// One pass of the watchdog. Returns how long until time alone could change a decision.
pub(super) fn stall_tick(reg: &Arc<Registry>) -> Option<Duration> {
    let threshold = reg.with_settings(crate::stall::threshold_ms);
    let minutes = crate::stall::display_minutes(threshold);
    let picked: Vec<Picked> = {
        let l = reg.sessions.lock_recover();
        let labels = reg.labels.lock_recover();
        let metas = reg.metas.lock_recover();
        l.iter().map(|(id, s)| (*id, s.clone(), s.view(), labels.get(id).cloned(), metas.get(id).cloned())).collect()
    };
    // the processes each running agent started, so a background shell or a build is not mistaken for silence
    if threshold > 0 {
        let agents: Vec<_> =
            picked.iter().filter(|(_, s, _, _, m)| m.as_ref().is_some_and(|m| m["kind"] == "agent") && s.try_wait().is_none()).collect();
        if !agents.is_empty() {
            let snap = axelstra_core::procs::snapshot_cached(Duration::from_secs(5));
            let now = Instant::now();
            for (id, s, ..) in agents {
                let Some(root) = s.pid() else { continue };
                let mut below = snap.descendants(&[root]);
                below.remove(&root);
                reg.work.observe(*id, now, &below);
            }
        }
    }
    reg.work.retain(&picked.iter().map(|p| p.0).collect());
    let busy = busy_parents(reg, &picked);
    let mut stalled = Vec::new();
    let mut human = std::collections::HashSet::new();
    let mut worked = std::collections::HashSet::new();
    let mut next: Option<u64> = None;
    let mut soonest = |ms: Option<u64>| {
        if let Some(ms) = ms {
            next = Some(next.map_or(ms, |n| n.min(ms)));
        }
    };
    let now_ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64);
    for (id, s, view, label, meta) in &picked {
        let facts = stall_facts(reg, *id, s.try_wait().is_none(), view.idle_ms(), view.since_input_ms(), meta.as_ref(), &busy);
        let await_left = reg.awaiting_of(*id, view.since_input_ms(), view.idle_ms()).map(|a| a.until.saturating_sub(now_ms));
        soonest(crate::stall::next_check_ms(&facts, threshold, await_left));
        soonest(crate::stall::next_human_ms(&facts, threshold));
        if facts.quiet_ms() < threshold {
            worked.insert(*id);
        }
        if crate::stall::is_stalled(&facts, threshold) {
            let parent = meta.as_ref().and_then(|m| m["parent"].as_u64()).map(|p| p as u32);
            stalled.push((*id, label.clone(), parent, view.screen_tail(12)));
            if crate::stall::needs_human(&facts, threshold) {
                human.insert(*id);
            }
        }
    }
    let fresh: Vec<_> = {
        let alive: std::collections::HashSet<u32> = picked.iter().map(|p| p.0).collect();
        let mut seen = reg.stall_notified.lock_recover();
        // an episode ends when the session works again (output, typing, a message) or goes away
        seen.retain(|id, _| alive.contains(id) && !worked.contains(id));
        let now = Instant::now();
        // the first look of an episode stamps the time the parent was told; a later look must not move it, or any pulse
        // (output, a message, a resource sample) would restart the wait before the user is alerted
        stalled.into_iter().filter(|x| seen.get(&x.0).is_none() && seen.insert(x.0, now).is_none()).collect()
    };
    if !fresh.is_empty() {
        // the alert of a session with a parent agent falls due a threshold after the parent was told
        soonest(Some(threshold + 20));
    }
    // one message per parent and pass, however many of its children stalled together
    type Child = (String, u32, Vec<String>);
    let mut by_parent: Vec<(u32, Vec<Child>)> = Vec::new();
    for (id, label, parent, screen) in fresh {
        let Some(pid) = parent.filter(|p| reg.get(*p).is_some()) else { continue };
        let item = (label.unwrap_or_else(|| format!("session {id}")), id, screen);
        match by_parent.iter_mut().find(|(p, _)| *p == pid) {
            Some((_, v)) => v.push(item),
            None => by_parent.push((pid, vec![item])),
        }
    }
    for (pid, items) in by_parent {
        let Some(ps) = reg.get(pid) else { continue };
        let text = match items.as_slice() {
            [(label, id, screen)] => crate::stall::parent_text(label, *id, minutes, &crate::stall::last_line(screen)),
            _ => crate::stall::parent_text_many(&items.iter().map(|(l, id, _)| (l.clone(), *id)).collect::<Vec<_>>(), minutes),
        };
        // a `notice` leaves the parent's own state (its pending question, its report) alone
        let from = if let [(_, id, _)] = items.as_slice() { Some(*id) } else { None };
        let m = reg.bus.send(from, pid, "notice", &text, None, None);
        let agent = reg.metas.lock_recover().get(&pid).map(|m| m["kind"] == "agent").unwrap_or(false);
        if agent {
            reg.deliver_untrusted(pid, ps, &crate::msgbus::notice(&m, "axelstra watchdog"));
        }
    }
    // the user's alert moved: watchers learn it now (the session monitor does not look at stalls)
    let changed = std::mem::replace(&mut *reg.work.human.lock_recover(), human.clone()) != human;
    if changed {
        reg.broadcast("session.changed");
    }
    next.map(Duration::from_millis)
}

#[cfg(test)]
mod usage_roots_tests {
    use super::*;

    fn event(path: &str) -> notify::Event {
        notify::Event::new(notify::EventKind::Modify(notify::event::ModifyKind::Any)).add_path(PathBuf::from(path))
    }

    #[test]
    fn an_event_matches_by_the_root_as_spelled_or_by_its_resolved_form() {
        let kinds = UsageKinds { usage: true, limits: false };
        let roots = vec![UsageRoot { dir: PathBuf::from("/var/agent"), kinds, json_only: false }];
        // the resolved form comes from the cache: no file system call decides the match
        let canon = vec![Some(PathBuf::from("/private/var/agent"))];
        assert_eq!(usage_kinds_of(&event("/var/agent/a.jsonl"), &roots, &canon), kinds);
        assert_eq!(usage_kinds_of(&event("/private/var/agent/a.jsonl"), &roots, &canon), kinds);
        assert_eq!(usage_kinds_of(&event("/elsewhere/a.jsonl"), &roots, &canon), UsageKinds::default());
        // a root that did not exist yet has no resolved form and matches as spelled only
        assert_eq!(usage_kinds_of(&event("/private/var/agent/a.jsonl"), &roots, &[None]), UsageKinds::default());
    }

    #[test]
    fn json_only_roots_ignore_other_files() {
        let kinds = UsageKinds { usage: false, limits: true };
        let roots = vec![UsageRoot { dir: PathBuf::from("/d/status"), kinds, json_only: true }];
        assert_eq!(usage_kinds_of(&event("/d/status/s.json"), &roots, &[None]), kinds);
        assert_eq!(usage_kinds_of(&event("/d/status/s.tmp"), &roots, &[None]), UsageKinds::default());
    }

    #[test]
    fn resolving_a_root_happens_once_and_missing_roots_are_none() {
        let r = UsageRoot { dir: std::env::temp_dir().join("axl-no-such-root-xyz"), kinds: UsageKinds::default(), json_only: false };
        assert_eq!(resolve_root(&r), None);
        let r = UsageRoot { dir: std::env::temp_dir(), kinds: UsageKinds::default(), json_only: false };
        assert!(resolve_root(&r).is_some());
    }
}
