//! Pausing under memory pressure: which processes to freeze at Critical, the table of what is frozen, and the
//! guarantees that nothing stays frozen.
//!
//! Suspending does not give memory back: it stops the growth, and lets the OS page the frozen processes out. Victims go in
//! this order, one at a time, until the level drops (a level change wakes the loop at once; `SETTLE` is only the longest it
//! waits to see whether the last pause helped):
//!   1. builds and tests running under `axelstra lock` (they register with `mem.hold`): cheapest to freeze, nobody talks to them;
//!   2. idle workers (ephemeral, quiet for `IDLE_WORKER_MS`, no input for 2 minutes);
//!   3. other agents, 4. leads, 5. the project main: each idle for longer, and only when nothing earlier is left.
//!
//! Never: the daemon, a terminal (a shell may host a server the user runs), a session the user typed into in the last 2 minutes.
//!
//! The network connections of a frozen agent may time out, which is why builds go first.
//!
//! Nothing stays frozen forever: the pause loop resumes by event when the level leaves Critical (or the governor / auto-pause
//! is switched off), `mem.resume` and a session kill/remove/exit resume first, a daemon shutdown, a `Drop` guard on `serve`
//! and the panic hook resume everything, and a daemon that crashed while processes were frozen finds `paused.json` at start
//! and continues the pids it lists (checking pid and start time, so a reused pid is left alone).
use super::*;
use crate::governor::suspend::Suspender;
use crate::governor::Level;
use std::collections::HashSet;

const MB: u64 = 1024 * 1024;
/// A session the user typed into this recently is never paused.
pub(super) const RECENT_INPUT_MS: u64 = 2 * 60_000;
/// Quiet time (no output) before a worker counts as idle, an agent, a lead or the main.
pub(super) const IDLE_WORKER_MS: u64 = 20_000;
pub(super) const IDLE_AGENT_MS: u64 = 60_000;
pub(super) const IDLE_LEAD_MS: u64 = 5 * 60_000;
/// Pausing a session that holds less than this frees nothing worth the interruption.
pub(super) const MIN_RSS_MB: u64 = 32;
/// The longest the loop waits after a pause to see whether the level dropped, before it pauses the next victim.
const SETTLE: Duration = Duration::from_secs(2);
/// A wait that only exists so the loop never blocks without a bound; levels wake it long before.
const BACKSTOP: Duration = Duration::from_secs(3600);

/// Why a thing is paused: the governor, or the user (`mem.pause`). Only `MEMORY` is resumed by a level change.
pub(super) const MEMORY: &str = "memory";
pub(super) const MANUAL: &str = "manual";

/// What can be paused: a whole session's process tree, or the build/test `axelstra lock` runs (by hold id).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Target {
    Session(u32),
    Build(u64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Role {
    Build,
    Worker,
    Agent,
    Lead,
    Main,
    Terminal,
}

/// What the victim choice needs to know about one candidate.
#[derive(Clone, Copy, Debug)]
pub(super) struct Facts {
    pub target: Target,
    pub role: Role,
    pub rss_mb: u64,
    pub since_input_ms: u64,
    pub idle_ms: u64,
}

fn tier(r: Role) -> u8 {
    match r {
        Role::Build => 0,
        Role::Worker => 1,
        Role::Agent => 2,
        Role::Lead => 3,
        Role::Main => 4,
        Role::Terminal => u8::MAX,
    }
}

/// Whether `f` may be paused at all.
pub(super) fn eligible(f: &Facts) -> bool {
    match f.role {
        Role::Terminal => false,
        // a build is not interactive: the user's typing in its session says nothing about it
        Role::Build => true,
        r => {
            let idle = match r {
                Role::Worker => IDLE_WORKER_MS,
                Role::Agent => IDLE_AGENT_MS,
                _ => IDLE_LEAD_MS,
            };
            f.rss_mb >= MIN_RSS_MB && f.since_input_ms >= RECENT_INPUT_MS && f.idle_ms >= idle
        }
    }
}

fn target_key(t: Target) -> (u8, u64) {
    match t {
        Target::Build(i) => (0, i),
        Target::Session(i) => (1, u64::from(i)),
    }
}

/// The eligible candidates in the order to pause them: by tier, then the biggest first. Pure.
pub(super) fn victims(c: &[Facts]) -> Vec<Target> {
    let mut v: Vec<&Facts> = c.iter().filter(|f| eligible(f)).collect();
    v.sort_by(|a, b| tier(a.role).cmp(&tier(b.role)).then(b.rss_mb.cmp(&a.rss_mb)).then(target_key(a.target).cmp(&target_key(b.target))));
    v.into_iter().map(|f| f.target).collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Pid {
    pid: u32,
    start: Option<u64>,
}

struct Entry {
    target: Target,
    reason: &'static str,
    since: u64,
    pids: Vec<Pid>,
}

/// A build or test that `axelstra lock` runs (registered with `mem.hold`).
#[derive(Clone, Debug)]
pub(super) struct Hold {
    pub name: String,
    /// the session it runs in
    pub session: u32,
    pub pid: u32,
    pub start: Option<u64>,
}

#[derive(Default)]
struct PState {
    entries: Vec<Entry>,
    holds: HashMap<u64, Hold>,
    next_hold: u64,
    seq: u64,
}

/// The table of what is frozen, and of the builds that could be. A leaf lock: nothing else is taken while it is held.
pub(super) struct Pauser {
    st: Mutex<PState>,
    suspender: Arc<dyn Suspender>,
    /// `paused.json`: the pids that are frozen, so a crash can be undone at the next start. (last sequence written)
    file: Mutex<u64>,
    path: std::path::PathBuf,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

impl Pauser {
    pub(super) fn new(suspender: Arc<dyn Suspender>, path: std::path::PathBuf) -> Pauser {
        Pauser { st: Mutex::new(PState::default()), suspender, file: Mutex::new(0), path }
    }

    /// The pids frozen now with their start times, taken under the lock with a sequence number.
    fn snapshot(g: &mut PState) -> (u64, Vec<Pid>) {
        g.seq += 1;
        (g.seq, g.entries.iter().flat_map(|e| e.pids.iter().copied()).collect())
    }

    fn save(&self, (seq, pids): (u64, Vec<Pid>)) {
        let mut last = self.file.lock_recover();
        if seq < *last {
            return;
        }
        *last = seq;
        if pids.is_empty() {
            let _ = std::fs::remove_file(&self.path);
        } else {
            let list: Vec<Value> = pids.iter().map(|p| json!({"pid": p.pid, "start": p.start})).collect();
            let _ = crate::store::write_json_with(&self.path, &list, false);
        }
    }

    /// Freezes `pids` as one entry. False when the target is already frozen or nothing could be stopped.
    pub(super) fn pause(&self, target: Target, reason: &'static str, pids: &[u32]) -> bool {
        let snap = {
            let mut g = self.st.lock_recover();
            if g.entries.iter().any(|e| e.target == target) {
                return false;
            }
            let taken: HashSet<u32> = g.entries.iter().flat_map(|e| e.pids.iter().map(|p| p.pid)).collect();
            let want: Vec<u32> = pids.iter().copied().filter(|p| !taken.contains(p)).collect();
            // starts first: a process read after it was stopped is the same process, but this is cheaper to reason about
            let starts: Vec<(u32, Option<u64>)> = want.iter().map(|p| (*p, origin::process_start(*p))).collect();
            let stopped = self.suspender.suspend(&want);
            if stopped.is_empty() {
                return false;
            }
            let pids = starts.into_iter().filter(|(p, _)| stopped.contains(p)).map(|(pid, start)| Pid { pid, start }).collect();
            g.entries.push(Entry { target, reason, since: now_ms(), pids });
            Self::snapshot(&mut g)
        };
        self.save(snap);
        true
    }

    /// Continues and forgets every entry `pred` picks. Returns their targets.
    fn release(&self, pred: impl Fn(&Entry) -> bool) -> Vec<Target> {
        let (out, snap) = {
            let mut g = self.st.lock_recover();
            let (gone, kept): (Vec<Entry>, Vec<Entry>) = std::mem::take(&mut g.entries).into_iter().partition(|e| pred(e));
            g.entries = kept;
            let mut out = Vec::new();
            for e in gone {
                let pids: Vec<u32> = e.pids.iter().map(|p| p.pid).collect();
                self.suspender.resume(&pids);
                out.push(e.target);
            }
            (out, Self::snapshot(&mut g))
        };
        self.save(snap);
        out
    }

    /// Continues what is frozen for `target`.
    pub(super) fn resume_target(&self, target: Target) -> bool {
        !self.release(|e| e.target == target).is_empty()
    }

    /// Continues a session and the builds it runs.
    pub(super) fn resume_session(&self, id: u32) -> Vec<Target> {
        let builds: HashSet<u64> = self.st.lock_recover().holds.iter().filter(|(_, h)| h.session == id).map(|(i, _)| *i).collect();
        self.release(|e| match e.target {
            Target::Session(s) => s == id,
            Target::Build(b) => builds.contains(&b),
        })
    }

    /// Continues every entry paused for `reason`.
    pub(super) fn resume_reason(&self, reason: &str) -> Vec<Target> {
        self.release(|e| e.reason == reason)
    }

    pub(super) fn resume_all(&self) -> Vec<Target> {
        self.release(|_| true)
    }

    /// `(reason, since ms, is a build)` when session `id` is frozen or runs a frozen build.
    pub(super) fn paused_of(&self, id: u32) -> Option<(&'static str, u64, bool)> {
        let g = self.st.lock_recover();
        g.entries.iter().find_map(|e| match e.target {
            Target::Session(s) if s == id => Some((e.reason, e.since, false)),
            Target::Build(b) if g.holds.get(&b).is_some_and(|h| h.session == id) => Some((e.reason, e.since, true)),
            _ => None,
        })
    }

    pub(super) fn is_paused(&self, target: Target) -> bool {
        self.st.lock_recover().entries.iter().any(|e| e.target == target)
    }

    /// Everything frozen, for `mem.status`.
    pub(super) fn list(&self) -> Vec<Value> {
        let g = self.st.lock_recover();
        g.entries
            .iter()
            .map(|e| match e.target {
                Target::Session(id) => {
                    json!({"kind": "session", "session": id, "reason": e.reason, "since": e.since, "pids": e.pids.len()})
                }
                Target::Build(b) => {
                    let h = g.holds.get(&b);
                    json!({"kind": "build", "hold": b, "session": h.map(|h| h.session), "name": h.map(|h| h.name.clone()),
                           "reason": e.reason, "since": e.since, "pids": e.pids.len()})
                }
            })
            .collect()
    }

    /// Registers a build running under `axelstra lock`.
    pub(super) fn hold(&self, name: &str, session: u32, pid: u32) -> u64 {
        let mut g = self.st.lock_recover();
        g.next_hold += 1;
        let id = g.next_hold;
        g.holds.insert(id, Hold { name: name.to_string(), session, pid, start: origin::process_start(pid) });
        id
    }

    /// The build ended: forgotten, and continued if it was frozen (its process is about to exit, or already has).
    pub(super) fn unhold(&self, id: u64) -> bool {
        let known = self.st.lock_recover().holds.remove(&id).is_some();
        self.resume_target(Target::Build(id));
        known
    }

    pub(super) fn holds(&self) -> Vec<(u64, Hold)> {
        self.st.lock_recover().holds.iter().map(|(i, h)| (*i, h.clone())).collect()
    }

    /// Start of the daemon: the pids a crashed run left frozen are continued if they are still the same processes.
    pub(super) fn recover_stale(&self) -> usize {
        let Ok(Some(list)) = crate::store::read_json::<Vec<Value>>(&self.path) else { return 0 };
        let mut n = 0;
        for v in list {
            let (Some(pid), Some(start)) = (v["pid"].as_u64().and_then(|p| u32::try_from(p).ok()), v["start"].as_u64()) else { continue };
            if origin::process_start(pid) == Some(start) {
                self.suspender.resume(&[pid]);
                n += 1;
            }
        }
        let _ = std::fs::remove_file(&self.path);
        n
    }
}

/// Dropped when `serve` ends, however it ends: nothing stays frozen behind a daemon that is gone.
pub(super) struct ResumeGuard(pub Arc<Registry>);

impl Drop for ResumeGuard {
    fn drop(&mut self) {
        self.0.resume_paused(None);
        crate::governor::suspend::resume_all_tracked();
    }
}

impl Registry {
    /// Pauses a session's whole process tree. False when it is already paused, not running, or nothing could be stopped.
    pub(super) fn pause_session(&self, id: u32, reason: &'static str) -> bool {
        let Some(s) = self.get(id).filter(|s| s.try_wait().is_none()) else { return false };
        let Some(root) = s.pid() else { return false };
        let tree: Vec<u32> = axelstra_core::procs::snapshot().descendants(&[root]).into_iter().collect();
        if !self.pauser.pause(Target::Session(id), reason, &tree) {
            return false;
        }
        self.announce_pause(Target::Session(id), reason);
        true
    }

    /// Pauses the build registered as hold `hold`.
    pub(super) fn pause_build(&self, hold: u64, reason: &'static str) -> bool {
        let Some(h) = self.pauser.holds().into_iter().find(|(i, _)| *i == hold).map(|(_, h)| h) else { return false };
        // a hold whose process is gone (or whose pid now belongs to another one) is not ours to touch
        if h.start.is_none() || origin::process_start(h.pid) != h.start {
            return false;
        }
        let tree: Vec<u32> = axelstra_core::procs::snapshot().descendants(&[h.pid]).into_iter().collect();
        if !self.pauser.pause(Target::Build(hold), reason, &tree) {
            return false;
        }
        self.announce_pause(Target::Build(hold), reason);
        true
    }

    /// Continues what was paused for `reason` (`None`: everything) and tells the watchers.
    pub(super) fn resume_paused(&self, reason: Option<&str>) -> Vec<Target> {
        let t = match reason {
            Some(r) => self.pauser.resume_reason(r),
            None => self.pauser.resume_all(),
        };
        self.announce_resume(&t);
        t
    }

    /// Continues a session before it is killed or removed (and the builds it runs).
    pub(super) fn resume_before_kill(&self, id: u32) {
        let t = self.pauser.resume_session(id);
        self.announce_resume(&t);
    }

    fn announce_pause(&self, t: Target, reason: &str) {
        let (session, name) = match t {
            Target::Session(id) => (Some(id), None),
            Target::Build(b) => {
                let h = self.pauser.holds().into_iter().find(|(i, _)| *i == b).map(|(_, h)| h);
                (h.as_ref().map(|h| h.session), h.map(|h| h.name))
            }
        };
        self.broadcast_event(json!({"event": "session.paused", "id": session, "build": name, "reason": reason}));
        self.pulse.bump();
    }

    pub(super) fn announce_resumed(&self, t: &[Target]) {
        self.announce_resume(t);
    }

    fn announce_resume(&self, t: &[Target]) {
        if t.is_empty() {
            return;
        }
        for t in t {
            let session = match t {
                Target::Session(id) => Some(*id),
                Target::Build(b) => self.pauser.holds().into_iter().find(|(i, _)| i == b).map(|(_, h)| h.session),
            };
            self.broadcast_event(json!({"event": "session.resumed", "id": session}));
        }
        self.pulse.bump();
    }

    /// The candidates for pausing now, with what the choice needs to know. Builds whose process is gone are dropped.
    pub(super) fn pause_candidates(&self) -> Vec<Facts> {
        let picked: Vec<_> = {
            let sessions = self.sessions.lock_recover();
            let metas = self.metas.lock_recover();
            sessions
                .iter()
                .filter(|(_, s)| s.try_wait().is_none())
                .map(|(id, s)| (*id, s.clone(), metas.get(id).cloned().unwrap_or(Value::Null)))
                .collect()
        };
        let snap = axelstra_core::procs::snapshot_cached(Duration::from_millis(800));
        let mut out: Vec<Facts> = picked
            .into_iter()
            .filter(|(id, ..)| !self.pauser.is_paused(Target::Session(*id)))
            .map(|(id, s, meta)| {
                let role = if meta["kind"] != "agent" {
                    Role::Terminal
                } else if meta["main"] == true {
                    Role::Main
                } else if meta["lead"] == true {
                    Role::Lead
                } else if meta["worker"] == true {
                    Role::Worker
                } else {
                    Role::Agent
                };
                let view = s.view();
                Facts {
                    target: Target::Session(id),
                    role,
                    rss_mb: s.memory_now_in(&snap) / MB,
                    since_input_ms: view.since_input_ms(),
                    idle_ms: view.idle_ms(),
                }
            })
            .collect();
        for (id, h) in self.pauser.holds() {
            if self.pauser.is_paused(Target::Build(id)) || h.start.is_none() || origin::process_start(h.pid) != h.start {
                continue;
            }
            let tree = snap.descendants(&[h.pid]);
            let kib: u64 = snap.procs.iter().filter(|p| tree.contains(&p.pid)).map(|p| p.rss_kib).sum();
            out.push(Facts {
                target: Target::Build(id),
                role: Role::Build,
                rss_mb: kib / 1024,
                since_input_ms: u64::MAX,
                idle_ms: u64::MAX,
            });
        }
        out
    }
}

/// One reaction to the level: Critical pauses (when it may), any other level lets the memory pauses go.
pub(super) fn react(reg: &Arc<Registry>, level: Level) {
    let cfg = reg.governor.config();
    if level == Level::Critical && cfg.pause_active() {
        round(reg);
        return;
    }
    if level == Level::Critical && cfg.enabled && cfg.auto_pause && cfg.warn_only {
        let would: Vec<Value> = victims(&reg.pause_candidates())
            .into_iter()
            .map(|t| match t {
                Target::Session(id) => json!({"session": id}),
                Target::Build(b) => json!({"build": b}),
            })
            .collect();
        reg.broadcast_event(json!({"event": "memory.would_pause", "targets": would}));
        return;
    }
    reg.resume_paused(Some(MEMORY));
}

/// The sessions to try to hibernate, in order: workers, agents, leads; the biggest first within a tier. Pure.
pub(super) fn sleepers(c: &[Facts]) -> Vec<u32> {
    let mut v: Vec<&Facts> = c
        .iter()
        .filter(|f| matches!(f.target, Target::Session(_)) && !matches!(f.role, Role::Terminal | Role::Main | Role::Build))
        .collect();
    v.sort_by(|a, b| tier(a.role).cmp(&tier(b.role)).then(b.rss_mb.cmp(&a.rss_mb)).then(target_key(a.target).cmp(&target_key(b.target))));
    v.into_iter()
        .filter_map(|f| match f.target {
            Target::Session(id) => Some(id),
            Target::Build(_) => None,
        })
        .collect()
}

/// Pauses victims one by one while the level stays Critical.
fn round(reg: &Arc<Registry>) {
    loop {
        let cfg = reg.governor.config();
        if reg.closing.load(Ordering::SeqCst) || reg.governor.level() != Level::Critical || !cfg.pause_active() {
            return;
        }
        let candidates = reg.pause_candidates();
        let order = victims(&candidates);
        // hibernating ends the process tree and frees the memory; pausing only stops its growth. So a session that can
        // sleep goes to sleep first, one per round. `session.hibernate` makes the real checks (idle, resumable, no
        // command running), so the pool is wider than the pause victims: every agent, workers first, biggest first
        let mut paused = reg.with_settings(crate::hibernate::policy).enabled
            && sleepers(&candidates).into_iter().any(|id| super::hibernate::hibernate(reg, id).is_ok());
        for t in order {
            if paused {
                break;
            }
            paused = match t {
                Target::Session(id) => reg.pause_session(id, MEMORY),
                Target::Build(b) => reg.pause_build(b, MEMORY),
            };
            if paused {
                break;
            }
        }
        if !paused {
            return;
        }
        // a level change (or a config poke) ends the wait at once
        let seen = reg.governor.generation();
        reg.governor.wait_change(seen, SETTLE);
    }
}

/// The thread that reacts to level changes. Sleeps on the governor's own change signal (no tick).
pub(super) fn spawn(reg: &Arc<Registry>) {
    let weak = Arc::downgrade(reg);
    let gov = reg.governor.clone();
    let _ = std::thread::Builder::new().name("governor-pause".into()).spawn(move || {
        // the level may have moved before this thread got going: look at it once
        let mut seen = gov.generation();
        if let Some(reg) = weak.upgrade() {
            react(&reg, gov.level());
        }
        loop {
            let (level, gen) = gov.wait_change(seen, BACKSTOP);
            seen = gen;
            if gov.is_closed() {
                return;
            }
            let Some(reg) = weak.upgrade() else { return };
            if reg.closing.load(Ordering::SeqCst) {
                return;
            }
            react(&reg, level);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::governor::suspend::fake::FakeSuspender;
    use crate::governor::MemInfo;

    fn f(target: Target, role: Role, rss_mb: u64, since_input_ms: u64, idle_ms: u64) -> Facts {
        Facts { target, role, rss_mb, since_input_ms, idle_ms }
    }
    fn s(id: u32) -> Target {
        Target::Session(id)
    }
    const QUIET: u64 = 10 * 60_000;

    #[test]
    fn sleepers_are_agents_by_tier_then_size_and_never_main_terminals_or_builds() {
        let c = [
            f(s(1), Role::Main, 900, QUIET, QUIET),
            f(s(2), Role::Lead, 800, QUIET, QUIET),
            f(s(3), Role::Agent, 700, 1_000, 1_000),
            f(s(4), Role::Worker, 100, QUIET, QUIET),
            f(s(5), Role::Worker, 400, QUIET, QUIET),
            f(s(6), Role::Terminal, 4000, QUIET, QUIET),
            f(Target::Build(7), Role::Build, 600, u64::MAX, u64::MAX),
        ];
        // not filtered by the pause rules (recent input, idle time): `session.hibernate` makes its own checks
        assert_eq!(sleepers(&c), [5, 4, 3, 2]);
    }

    #[test]
    fn victim_priority_order() {
        let c = [
            f(s(1), Role::Main, 900, QUIET, QUIET),
            f(s(2), Role::Lead, 800, QUIET, QUIET),
            f(s(3), Role::Agent, 700, QUIET, QUIET),
            f(s(4), Role::Worker, 100, QUIET, QUIET),
            f(s(5), Role::Worker, 400, QUIET, QUIET),
            f(Target::Build(7), Role::Build, 50, u64::MAX, u64::MAX),
            f(Target::Build(8), Role::Build, 600, u64::MAX, u64::MAX),
        ];
        assert_eq!(victims(&c), [Target::Build(8), Target::Build(7), s(5), s(4), s(3), s(2), s(1)]);
    }

    #[test]
    fn never_victims() {
        // a terminal, whatever it holds; a session typed into 30 s ago; one still printing; one holding almost nothing
        let c = [
            f(s(1), Role::Terminal, 4000, QUIET, QUIET),
            f(s(2), Role::Worker, 500, 30_000, QUIET),
            f(s(3), Role::Worker, 500, QUIET, 1_000),
            f(s(4), Role::Worker, MIN_RSS_MB - 1, QUIET, QUIET),
            f(s(5), Role::Agent, 500, QUIET, 30_000),
            f(s(6), Role::Lead, 500, QUIET, 2 * 60_000),
            f(s(7), Role::Main, 500, 119_999, QUIET),
        ];
        assert!(victims(&c).is_empty(), "{:?}", victims(&c));
        // a build is eligible even when its session was typed into a second ago
        assert_eq!(victims(&[f(Target::Build(1), Role::Build, 0, 1_000, 0)]), [Target::Build(1)]);
        // the same worker once it is quiet and the user is away
        assert_eq!(victims(&[f(s(2), Role::Worker, 500, 120_000, 20_000)]), [s(2)]);
    }

    fn pauser() -> (Arc<FakeSuspender>, Pauser, std::path::PathBuf) {
        static N: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!("axl-pause-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let fake = Arc::new(FakeSuspender::default());
        let p = Pauser::new(fake.clone(), dir.join("paused.json"));
        (fake, p, dir)
    }

    #[test]
    fn pause_and_resume_track_pids_and_the_state_file() {
        let (fake, p, dir) = pauser();
        let me = std::process::id();
        // our own process stands in for a pid with a readable start time; the fake stops nothing real
        assert!(p.pause(s(1), MEMORY, &[me]));
        assert!(fake.is_stopped(me));
        assert!(dir.join("paused.json").exists());
        // the same target, and a pid that is already frozen, are not frozen twice
        assert!(!p.pause(s(1), MEMORY, &[me]));
        assert!(!p.pause(s(2), MEMORY, &[me]));
        assert!(p.paused_of(1).is_some() && p.paused_of(2).is_none());
        assert_eq!(p.resume_reason(MANUAL), []);
        assert_eq!(p.resume_reason(MEMORY), [s(1)]);
        assert!(!fake.is_stopped(me));
        assert!(!dir.join("paused.json").exists(), "an empty table leaves no file");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_stale_state_file_is_undone_at_start_but_only_for_the_same_process() {
        let (fake, p, dir) = pauser();
        let me = std::process::id();
        let start = origin::process_start(me).unwrap();
        // a crashed run left two entries: our pid with its real start time, and the same pid with a start time it never had
        std::fs::write(
            dir.join("paused.json"),
            json!({"v": 1, "data": [{"pid": me, "start": start}, {"pid": 4_000_000, "start": 1}, {"pid": me, "start": start + 1}]})
                .to_string(),
        )
        .unwrap();
        assert_eq!(p.recover_stale(), 1);
        let log = fake.log.lock_recover().clone();
        assert_eq!(log, [format!("resume [{me}]")]);
        assert!(!dir.join("paused.json").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn reg_with(fake: Arc<FakeSuspender>) -> (Arc<Registry>, std::path::PathBuf) {
        static N: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!("axl-pausereg-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut r = Registry::new(dir.join("projects.json"), "p".into(), "t".into());
        r.pauser = Pauser::new(fake, dir.join("paused.json"));
        (Arc::new(r), dir)
    }

    /// Wait for `cond`, by short sleeps only inside a test.
    fn until(what: &str, cond: impl Fn() -> bool) {
        let t = Instant::now();
        while !cond() {
            assert!(t.elapsed() < Duration::from_secs(20), "timed out: {what}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn mem(free_mb: u64) -> MemInfo {
        MemInfo { total_mb: 16384, free_mb, commit_pct: None }
    }

    /// A registry with one agent worker session that is eligible (quiet, big enough is faked by a build instead).
    #[test]
    fn critical_pauses_the_build_and_leaving_critical_resumes_it_by_event() {
        let fake = Arc::new(FakeSuspender::default());
        let (reg, dir) = reg_with(fake.clone());
        // a real child of this test stands in for the build; the fake never signals it
        let mut child = std::process::Command::new(if cfg!(windows) { "ping" } else { "sleep" })
            .args(if cfg!(windows) { vec!["-n", "60", "127.0.0.1"] } else { vec!["60"] })
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let hold = reg.pauser.hold("verify", 1, child.id());
        spawn(&reg);
        reg.governor.feed(mem(8000));
        assert!(reg.governor.feed(mem(100)).is_some());
        until("the build is frozen", || fake.is_stopped(child.id()));
        assert!(reg.pauser.is_paused(Target::Build(hold)));
        assert_eq!(reg.pauser.paused_of(1).map(|p| (p.0, p.2)), Some((MEMORY, true)));
        // High is not Critical: memory pauses go
        assert!(reg.governor.feed(mem(2500)).is_some());
        until("the build runs again", || !fake.is_stopped(child.id()));
        assert!(reg.pauser.paused_of(1).is_none());
        reg.governor.stop();
        let _ = child.kill();
        let _ = child.wait();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn warn_only_pauses_nothing_and_a_manual_pause_survives_the_level() {
        let fake = Arc::new(FakeSuspender::default());
        let (reg, dir) = reg_with(fake.clone());
        let mut child = std::process::Command::new(if cfg!(windows) { "ping" } else { "sleep" })
            .args(if cfg!(windows) { vec!["-n", "60", "127.0.0.1"] } else { vec!["60"] })
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let hold = reg.pauser.hold("verify", 1, child.id());
        let cfg = reg.governor.config().patch(&json!({"warn_only": true})).unwrap();
        reg.governor.set_config(cfg, None);
        reg.governor.feed(mem(100));
        react(&reg, Level::Critical);
        assert!(!fake.is_stopped(child.id()), "warn-only must not pause");
        // the user's own pause is not undone by the level
        assert!(reg.pauser.pause(Target::Build(hold), MANUAL, &[child.id()]));
        react(&reg, Level::Normal);
        assert!(fake.is_stopped(child.id()), "a manual pause is only resumed by the user");
        reg.resume_paused(None);
        assert!(!fake.is_stopped(child.id()));
        reg.governor.stop();
        let _ = child.kill();
        let _ = child.wait();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn shutdown_guard_and_kill_resume_everything() {
        let fake = Arc::new(FakeSuspender::default());
        let (reg, dir) = reg_with(fake.clone());
        let me = std::process::id();
        assert!(reg.pauser.pause(s(5), MEMORY, &[me]));
        assert!(fake.is_stopped(me));
        // a kill/remove of that session resumes first
        reg.resume_before_kill(5);
        assert!(!fake.is_stopped(me));
        // the drop guard of `serve` and the shutdown path resume whatever is left
        assert!(reg.pauser.pause(s(6), MANUAL, &[me]));
        drop(ResumeGuard(reg.clone()));
        assert!(!fake.is_stopped(me));
        assert!(reg.pauser.list().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_build_that_ends_is_forgotten_and_continued() {
        let (fake, p, dir) = pauser();
        let me = std::process::id();
        let h = p.hold("verify", 3, me);
        assert!(p.pause(Target::Build(h), MEMORY, &[me]));
        assert!(p.unhold(h));
        assert!(!fake.is_stopped(me));
        assert!(!p.unhold(h));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
