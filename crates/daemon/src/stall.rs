//! Inactivity watchdog: an agent that went quiet without reporting (`done`/`await`/`ask`) is "stalled", not idle.
//! The decisions are pure functions of facts the registry gathers (`Facts`) and of the turn bookkeeping (`Turns`), so
//! they are tested without sessions.
//!
//! Two steps, both on events: the agent's own Stop hook (`session.hint` with `event: "stop"`) gets ONE short reminder
//! typed into that agent (`prompts::STOP_NUDGE`, `Turns::stop`); only when it stays quiet for the threshold after it
//! (or it has no Stop hook) does its parent hear, once per stall episode. A session with no parent agent is shown to the
//! user instead (`stalled` in `session.list`), once per episode too. A legitimately quiet session (`excused`) never
//! causes anything.

use serde_json::Value;
use std::time::{Duration, Instant};

/// Minutes of silence before an agent counts as stalled, unless the settings say otherwise (`stall_minutes`, 0 = off).
pub const DEFAULT_MINUTES: u64 = 15;
/// Upper bound accepted for the setting (a day).
pub const MAX_MINUTES: u64 = 1440;
/// At most one stop reminder per session in this long, whatever happens in between.
pub const NUDGE_EVERY: Duration = Duration::from_secs(10 * 60);

/// The configured threshold in minutes (0 = watchdog off). A missing or invalid value means the default.
pub fn minutes(saved: &Value) -> u64 {
    saved["stall_minutes"].as_u64().map(|m| m.min(MAX_MINUTES)).unwrap_or(DEFAULT_MINUTES)
}

/// The threshold in ms (0 = off). Tests shorten it with `AXELSTRA_STALL_SECS` (only honoured with `AXELSTRA_TESTING=1`).
pub fn threshold_ms(saved: &Value) -> u64 {
    if std::env::var_os("AXELSTRA_TESTING").is_some() {
        if let Some(s) = std::env::var("AXELSTRA_STALL_SECS").ok().and_then(|v| v.parse::<u64>().ok()) {
            return s * 1000;
        }
    }
    minutes(saved) * 60_000
}

/// Whole minutes for display (at least 1 while the watchdog is on).
pub fn display_minutes(threshold_ms: u64) -> u64 {
    if threshold_ms == 0 {
        0
    } else {
        (threshold_ms / 60_000).max(1)
    }
}

/// Validates a `settings.set` value for `stall_minutes`.
pub fn validate(v: &Value) -> Option<u64> {
    v.as_u64().filter(|m| *m <= MAX_MINUTES)
}

/// What the registry knows about one session.
#[derive(Clone, Copy, Debug, Default)]
pub struct Facts {
    /// an agent TUI (not a plain terminal)
    pub agent: bool,
    /// the project's main agent: the user's own conversation, never watched
    pub main: bool,
    /// the process is alive
    pub running: bool,
    /// it called `axelstra done`
    pub reported: bool,
    /// it called `axelstra escalate` and nobody answered yet (the parent already knows)
    pub escalated: bool,
    /// it waits for an answer: an unanswered `axelstra ask`, or its hook said permission/question
    pub waiting: bool,
    /// it said (`axelstra await`) that it only waits for something external, within its limit
    pub awaiting: bool,
    /// a child session of it is still running (it legitimately waits for it)
    pub live_children: bool,
    /// the memory governor froze it (or a build it runs): the silence is ours
    pub paused: bool,
    /// ms since the last output
    pub idle_ms: u64,
    /// ms since the user last typed into it
    pub since_input_ms: u64,
    /// ms since a message (a task, an answer, a notice) was last put in its inbox
    pub since_msg_ms: u64,
    /// a process of its tree that was not part of its start-up is alive: a background shell, a build, `axelstra lock`
    /// holding or waiting for a lock (see `Work`)
    pub bg_work: bool,
    /// it asked for a worker slot and none is free: it waits for one to end
    pub slot_wait: bool,
    /// its parent is a live agent session (Axelstra manages it: a worker, a lead, a task)
    pub parent_agent: bool,
    /// the user drives it (see `user_driven`): the user decides when it goes on
    pub user_driven: bool,
    /// it never got a request (see `never_asked`): it has nothing to stall on
    pub unasked: bool,
}

/// True for an agent started with no prompt, task or restored conversation (`meta.unrequested`, set at spawn) that
/// nobody has typed into or messaged since and whose hook never reported a turn: it only sits at its banner, waiting
/// for its first request.
pub fn never_asked(unrequested: bool, since_input_ms: u64, since_msg_ms: u64, hinted: bool) -> bool {
    unrequested && since_input_ms == u64::MAX && since_msg_ms == u64::MAX && !hinted
}

/// True when the user is the driver: the user typed into it, and either no agent is its parent or the user's typing is
/// more recent than the last message that reached it.
pub fn user_driven(parent_agent: bool, since_input_ms: u64, since_msg_ms: u64) -> bool {
    since_input_ms != u64::MAX && (!parent_agent || since_input_ms <= since_msg_ms)
}

impl Facts {
    /// ms of silence that count towards the threshold: no output, no typing and no message.
    pub fn quiet_ms(&self) -> u64 {
        self.idle_ms.min(self.since_input_ms).min(self.since_msg_ms)
    }
}

/// True when the session is legitimately quiet, or is not something the watchdog looks at: it never stalls and never
/// causes a notice, however long the silence.
pub fn excused(f: &Facts) -> bool {
    !f.agent
        || f.main
        || !f.running
        || f.reported
        || f.escalated
        || f.waiting
        || f.awaiting
        || f.live_children
        || f.paused
        || f.bg_work
        || f.slot_wait
        || f.user_driven
        || f.unasked
}

/// True when the agent has been quiet (no output, no typing, no message) for `threshold_ms` with nothing that explains
/// it (0 = off).
pub fn is_stalled(f: &Facts, threshold_ms: u64) -> bool {
    threshold_ms > 0 && !excused(f) && f.quiet_ms() >= threshold_ms
}

/// True when the agent's own turn end deserves the stop reminder: an Axelstra-managed agent (its parent is an agent)
/// that ended its turn with nothing that explains it (no `done`, `await`, `ask`/`escalate`/wait, live child, ...).
pub fn nudge_due(f: &Facts, threshold_ms: u64) -> bool {
    threshold_ms > 0 && f.parent_agent && !excused(f)
}

/// True when the parent must hear of the stall: stalled, with a parent agent, and the stop reminder (when one was typed
/// in this episode, `nudged_ms` ago) had a whole threshold to work. An agent with no Stop hook is never reminded.
pub fn parent_due(f: &Facts, threshold_ms: u64, nudged_ms: Option<u64>) -> bool {
    is_stalled(f, threshold_ms) && f.parent_agent && nudged_ms.is_none_or(|n| n >= threshold_ms)
}

/// True when the stall needs a person: nobody else handles it (no parent agent, and the user does not drive it).
pub fn needs_human(f: &Facts, threshold_ms: u64) -> bool {
    is_stalled(f, threshold_ms) && !f.parent_agent
}

/// Recheck period while a background process of the tree explains the silence: no OS event says a grandchild ended.
pub const BG_RECHECK_MS: u64 = 60_000;

/// When to look at this session again, in ms, if nothing happens meanwhile: the moment its quiet spell (and the time
/// since its stop reminder) reaches the threshold, or (while it awaits something) the moment the await expires.
/// `None`: nothing time alone can change, an event (pulse) will say. This is what lets the watchdog sleep instead of
/// ticking.
pub fn next_check_ms(f: &Facts, threshold_ms: u64, await_left_ms: Option<u64>, nudged_ms: Option<u64>) -> Option<u64> {
    if threshold_ms == 0
        || !f.agent
        || f.main
        || !f.running
        || f.reported
        || f.escalated
        || f.waiting
        || f.live_children
        || f.paused
        || f.user_driven
        || f.unasked
    {
        return None;
    }
    if f.awaiting {
        return await_left_ms.map(|l| l + 20);
    }
    if f.slot_wait {
        return None;
    }
    let quiet = f.quiet_ms();
    if quiet >= threshold_ms {
        if f.bg_work {
            return Some(BG_RECHECK_MS);
        }
        // stalled, but the reminder is still fresh: the parent is due when it has had a whole threshold
        return nudged_ms.filter(|n| *n < threshold_ms && f.parent_agent).map(|n| threshold_ms - n + 20);
    }
    Some(threshold_ms - quiet + 20)
}

/// Why a child's stall is told to its parent.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Why {
    /// it got the stop reminder and stayed quiet
    Nudged,
    /// it had a request but its hook never reported a turn: a trust dialog, a lost prompt or resume, or no hooks
    NeverStarted,
    /// its hook reported turns but no Stop reached the reminder (rate limit, an old CLI)
    Quiet,
}

/// The English line typed into the parent's session (agent-facing, so it stays English): one per stall episode.
pub fn parent_text(label: &str, id: u32, minutes: u64, why: Why) -> String {
    let what = match why {
        Why::Nudged => "stopped without done/await/ask and ignored one reminder".to_string(),
        Why::NeverStarted => format!("never reported a turn (a trust dialog or a lost prompt?) and has been quiet {minutes} min"),
        Why::Quiet => format!("stopped without done/await/ask and has been quiet {minutes} min"),
    };
    format!("\"{label}\" (session {id}) {what}. Check it: axelstra read {id}")
}

/// One line for several sessions of the same parent that stalled in the same pass: `(label, id)` each.
pub fn parent_text_many(items: &[(String, u32)]) -> String {
    let list: Vec<String> = items.iter().map(|(l, id)| format!("\"{l}\" (session {id})")).collect();
    format!("{} sessions stopped without done/await/ask: {}. Check each: axelstra read <session>", items.len(), list.join(", "))
}

#[derive(Default)]
struct Turn {
    /// the stop reminder of the current stop episode was typed then
    nudged: Option<Instant>,
    /// the last stop reminder, whatever the episode (the rate limit)
    last_nudge: Option<Instant>,
    /// the turn the reminder provoked is running: its working hints are the reminder, not new work
    echo: bool,
    /// the last working hint that was real work (a prompt submitted that was not the reminder)
    work: Option<Instant>,
    /// the stall episode began then (the parent was told, or the user alerted)
    stalled: Option<Instant>,
}

/// Per session: the stop reminder and the stall episode. An episode ends only on real new work (the user typed, a
/// message reached it, a working hint that is not the reminder's own turn), never on a stray output byte; `done` and the
/// other excuses are checked by the callers (`excused`).
#[derive(Default)]
pub struct Turns(std::sync::Mutex<std::collections::HashMap<u32, Turn>>);

/// Real new work since `at`: the user typed, a message arrived, or a working hint that was not the reminder.
fn worked_since(t: &Turn, at: Instant, now: Instant, since_input_ms: u64, since_msg_ms: u64) -> bool {
    let age = now.saturating_duration_since(at).as_millis() as u64;
    since_input_ms < age || since_msg_ms < age || t.work.is_some_and(|w| w > at)
}

impl Turns {
    /// The agent's hook said it works (a prompt submitted, a tool ran). During the turn the reminder provoked, it is not
    /// new work.
    pub fn working(&self, id: u32, now: Instant) {
        if let Ok(mut m) = self.0.lock() {
            let t = m.entry(id).or_default();
            if !t.echo {
                t.work = Some(now);
            }
        }
    }

    /// The agent's own Stop hook fired. Returns true when the reminder must be typed now (and records it): `due`
    /// (`nudge_due`), not reminded yet in this stop episode, and not reminded in the last `NUDGE_EVERY`.
    pub fn stop(&self, id: u32, now: Instant, since_input_ms: u64, since_msg_ms: u64, due: bool) -> bool {
        let Ok(mut m) = self.0.lock() else { return false };
        let t = m.entry(id).or_default();
        // the reminder's own turn (if any) ended here
        t.echo = false;
        if !due {
            return false;
        }
        if t.nudged.is_some_and(|n| !worked_since(t, n, now, since_input_ms, since_msg_ms)) {
            return false;
        }
        if t.last_nudge.is_some_and(|l| now.saturating_duration_since(l) < NUDGE_EVERY) {
            return false;
        }
        t.nudged = Some(now);
        t.last_nudge = Some(now);
        t.echo = true;
        true
    }

    /// ms since the reminder of the current stop episode (`None`: not reminded, or real work since).
    pub fn nudged_ms(&self, id: u32, now: Instant, since_input_ms: u64, since_msg_ms: u64) -> Option<u64> {
        let m = self.0.lock().ok()?;
        let t = m.get(&id)?;
        let n = t.nudged?;
        (!worked_since(t, n, now, since_input_ms, since_msg_ms)).then(|| now.saturating_duration_since(n).as_millis() as u64)
    }

    /// True while a stall episode is open: it began and no real work happened since.
    pub fn in_episode(&self, id: u32, now: Instant, since_input_ms: u64, since_msg_ms: u64) -> bool {
        let Ok(m) = self.0.lock() else { return false };
        m.get(&id).is_some_and(|t| t.stalled.is_some_and(|s| !worked_since(t, s, now, since_input_ms, since_msg_ms)))
    }

    /// The session is stalled: opens its episode. True only for the first call of an episode (the one that tells).
    pub fn begin_stall(&self, id: u32, now: Instant, since_input_ms: u64, since_msg_ms: u64) -> bool {
        let Ok(mut m) = self.0.lock() else { return false };
        let t = m.entry(id).or_default();
        if t.stalled.is_some_and(|s| !worked_since(t, s, now, since_input_ms, since_msg_ms)) {
            return false;
        }
        t.stalled = Some(now);
        true
    }

    /// Forgets a session.
    pub fn forget(&self, id: u32) {
        if let Ok(mut m) = self.0.lock() {
            m.remove(&id);
        }
    }

    /// Forgets the sessions not in `alive`.
    pub fn retain(&self, alive: &std::collections::HashSet<u32>) {
        if let Ok(mut m) = self.0.lock() {
            m.retain(|id, _| alive.contains(id));
        }
    }
}

/// The last meaningful line of a screen, trimmed and clipped (what the notification shows). A status bar or a bare
/// prompt ("⏵⏵ auto mode on", ">") says nothing about why the agent stopped, so it is skipped.
pub fn last_line(lines: &[String]) -> String {
    let l = lines
        .iter()
        .rev()
        .map(|l| l.trim())
        .find(|l| l.chars().next().is_some_and(char::is_alphanumeric) && l.chars().filter(|c| c.is_alphanumeric()).count() >= 8)
        .unwrap_or_default();
    l.chars().take(200).collect()
}

/// How long after a session's start every process in its tree counts as its own start-up (the agent's MCP servers
/// and helpers), not as work it started.
const BASE_WINDOW: std::time::Duration = std::time::Duration::from_secs(60);

struct Tree {
    born: std::time::Instant,
    base: std::collections::HashSet<u32>,
    busy: bool,
}

/// Work a quiet session legitimately has that no screen shows: processes it started (a background shell, a build, an
/// `axelstra lock` holding or waiting for a lock, `axelstra wait`) and a wish for a worker slot. State, not text.
#[derive(Default)]
pub struct Work {
    trees: std::sync::Mutex<std::collections::HashMap<u32, Tree>>,
    slots: std::sync::Mutex<std::collections::HashMap<u32, std::time::Instant>>,
    /// sessions whose stall currently needs the user (to tell watchers when it changes)
    pub human: std::sync::Mutex<std::collections::HashSet<u32>>,
}

impl Work {
    /// Records the processes below a session's own (`pids`, its root excluded) and returns whether one of them is new
    /// since the start-up window: a background process the session started.
    pub fn observe(&self, id: u32, now: std::time::Instant, pids: &std::collections::HashSet<u32>) -> bool {
        let Ok(mut m) = self.trees.lock() else { return false };
        let t = m.entry(id).or_insert_with(|| Tree { born: now, base: Default::default(), busy: false });
        if now.duration_since(t.born) < BASE_WINDOW {
            t.base.extend(pids.iter().copied());
        }
        t.busy = pids.iter().any(|p| !t.base.contains(p));
        t.busy
    }

    /// The last answer of `observe`.
    pub fn busy(&self, id: u32) -> bool {
        self.trees.lock().ok().and_then(|m| m.get(&id).map(|t| t.busy)).unwrap_or(false)
    }

    /// Forgets the sessions not in `alive`.
    pub fn retain(&self, alive: &std::collections::HashSet<u32>) {
        if let Ok(mut m) = self.trees.lock() {
            m.retain(|id, _| alive.contains(id));
        }
        if let Ok(mut m) = self.slots.lock() {
            m.retain(|id, _| alive.contains(id));
        }
        if let Ok(mut m) = self.human.lock() {
            m.retain(|id| alive.contains(id));
        }
    }

    /// A spawn of this session was refused for the worker limit: it waits for a slot (until it spawns, or the cap).
    pub fn want_slot(&self, id: u32) {
        if let Ok(mut m) = self.slots.lock() {
            m.entry(id).or_insert_with(std::time::Instant::now);
        }
    }

    /// It got to spawn (or gave up).
    pub fn got_slot(&self, id: u32) {
        if let Ok(mut m) = self.slots.lock() {
            m.remove(&id);
        }
    }

    /// True while the session wants a slot, none is free (`full`) and it asked less than the await cap ago.
    pub fn slot_waiting(&self, id: u32, full: impl FnOnce() -> bool) -> bool {
        let cap = u128::from(axelstra_core::waiting::AWAIT_CAP_MS);
        let asked = self.slots.lock().ok().and_then(|m| m.get(&id).map(|t| t.elapsed().as_millis() < cap)).unwrap_or(false);
        asked && full()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const TH: u64 = 300_000;

    fn quiet() -> Facts {
        Facts { agent: true, running: true, idle_ms: 6 * 60_000, since_input_ms: u64::MAX, since_msg_ms: u64::MAX, ..Facts::default() }
    }

    fn child() -> Facts {
        Facts { parent_agent: true, ..quiet() }
    }

    #[test]
    fn fires_after_the_threshold_only() {
        assert!(is_stalled(&quiet(), TH));
        assert!(!is_stalled(&Facts { idle_ms: 4 * 60_000, ..quiet() }, TH));
        assert!(is_stalled(&Facts { idle_ms: 4 * 60_000, ..quiet() }, 180_000));
    }

    #[test]
    fn typing_counts_as_activity() {
        assert!(!is_stalled(&Facts { since_input_ms: 10_000, ..quiet() }, TH));
        // frozen by the memory governor: the silence is not the agent's
        assert!(!is_stalled(&Facts { paused: true, ..quiet() }, 180_000));
        assert_eq!(next_check_ms(&Facts { paused: true, ..quiet() }, 180_000, None, None), None);
    }

    /// Every legitimately quiet case: never stalled, no reminder, no parent notice, no user alert.
    fn legit_quiet() -> Vec<(Facts, &'static str)> {
        let c = child();
        vec![
            (Facts { reported: true, ..c }, "done"),
            (Facts { escalated: true, ..c }, "escalate pending"),
            (Facts { waiting: true, ..c }, "ask pending or permission/question wait"),
            (Facts { awaiting: true, ..c }, "await"),
            (Facts { live_children: true, ..c }, "live child"),
            (Facts { paused: true, ..c }, "paused"),
            (Facts { bg_work: true, ..c }, "background process"),
            (Facts { slot_wait: true, ..c }, "waits for a worker slot"),
            (Facts { unasked: true, ..c }, "never asked"),
            (Facts { user_driven: true, ..c }, "user driven"),
            (Facts { user_driven: true, ..quiet() }, "user driven, no parent"),
            (Facts { running: false, ..c }, "exited"),
            (Facts { main: true, ..quiet() }, "project main"),
            (Facts { agent: false, ..quiet() }, "terminal"),
        ]
    }

    #[test]
    fn legitimately_quiet_sessions_never_cause_anything() {
        for (f, why) in legit_quiet() {
            assert!(excused(&f), "{why}");
            assert!(!is_stalled(&f, TH), "{why}");
            assert!(!nudge_due(&f, TH), "{why}");
            assert!(!parent_due(&f, TH, None), "{why}");
            assert!(!needs_human(&f, TH), "{why}");
        }
        assert!(!is_stalled(&quiet(), 0), "0 turns the watchdog off");
        assert!(!nudge_due(&child(), 0), "0 turns the reminder off too");
        // a message restarts the quiet spell
        assert!(!is_stalled(&Facts { since_msg_ms: 10_000, ..child() }, TH));
    }

    #[test]
    fn blank_agents_are_never_asked_until_someone_writes_or_a_turn_runs() {
        assert!(never_asked(true, u64::MAX, u64::MAX, false));
        assert!(!never_asked(true, 5_000, u64::MAX, false), "typed into");
        assert!(!never_asked(true, u64::MAX, 5_000, false), "got a message");
        assert!(!never_asked(true, u64::MAX, u64::MAX, true), "its hook reported a turn: it had a prompt");
        assert!(!never_asked(false, u64::MAX, u64::MAX, false), "not flagged: it had a request");
    }

    #[test]
    fn a_lost_task_is_not_blank() {
        // it had a request (a goal, a dispatch, a restored conversation): not flagged, so it follows the stall path
        let lost = Facts { unasked: never_asked(false, u64::MAX, u64::MAX, false), parent_agent: true, ..quiet() };
        assert!(parent_due(&lost, TH, None));
    }

    #[test]
    fn the_user_drives_what_the_user_typed_into_last() {
        assert!(!user_driven(false, u64::MAX, u64::MAX), "never typed");
        assert!(user_driven(false, 60_000, u64::MAX), "no parent agent, typed");
        assert!(user_driven(false, 60_000, 1_000), "no parent agent: typing ever is enough");
        assert!(user_driven(true, 60_000, u64::MAX), "a parent agent, but the user typed and no message came");
        assert!(user_driven(true, 1_000, 60_000), "the user typed after the last message");
        assert!(!user_driven(true, 60_000, 1_000), "a message came after the typing: the parent drives it again");
    }

    #[test]
    fn only_managed_agents_get_the_reminder_and_the_parent_waits_a_threshold_after_it() {
        assert!(nudge_due(&Facts { idle_ms: 0, ..child() }, TH), "turn end: due at once, whatever the quiet spell");
        assert!(!nudge_due(&quiet(), TH), "no parent agent: the user sees it");
        assert!(parent_due(&child(), TH, None), "no Stop hook: the quiet threshold is enough");
        assert!(!parent_due(&child(), TH, Some(60_000)), "the reminder is still fresh");
        assert!(parent_due(&child(), TH, Some(TH)), "it ignored the reminder for a whole threshold");
        assert!(!parent_due(&quiet(), TH, None), "no parent agent");
        assert!(needs_human(&quiet(), TH));
        assert!(!needs_human(&child(), TH), "a parent agent handles it, never the user");
    }

    #[test]
    fn next_check_is_the_moment_the_quiet_spell_reaches_the_threshold() {
        assert_eq!(next_check_ms(&Facts { idle_ms: 400_000, ..quiet() }, TH, None, None), None, "already stalled");
        let f = Facts { idle_ms: 100_000, since_input_ms: 250_000, ..child() };
        assert_eq!(next_check_ms(&f, TH, None, None), Some(200_020));
        assert_eq!(next_check_ms(&Facts { idle_ms: 0, ..quiet() }, TH, None, None), Some(300_020));
        // quiet long enough but reminded a minute ago: due when the reminder had a whole threshold
        assert_eq!(next_check_ms(&child(), TH, None, Some(60_000)), Some(240_020));
        assert_eq!(next_check_ms(&child(), TH, None, Some(TH)), None);
    }

    #[test]
    fn a_background_process_is_rechecked_by_a_bounded_deadline_a_slot_wait_by_event() {
        assert_eq!(next_check_ms(&Facts { bg_work: true, ..quiet() }, TH, None, None), Some(BG_RECHECK_MS));
        assert_eq!(next_check_ms(&Facts { slot_wait: true, ..quiet() }, TH, None, None), None);
        assert_eq!(next_check_ms(&Facts { since_msg_ms: 100_000, ..quiet() }, TH, None, None), Some(200_020));
    }

    #[test]
    fn next_check_ignores_what_only_an_event_can_change() {
        let f = Facts { idle_ms: 0, ..child() };
        for g in [
            Facts { reported: true, ..f },
            Facts { escalated: true, ..f },
            Facts { waiting: true, ..f },
            Facts { live_children: true, ..f },
            Facts { user_driven: true, ..f },
            Facts { unasked: true, ..f },
            Facts { running: false, ..f },
            Facts { main: true, ..f },
            Facts { agent: false, ..f },
        ] {
            assert_eq!(next_check_ms(&g, TH, None, None), None, "{g:?}");
        }
        assert_eq!(next_check_ms(&f, 0, None, None), None, "0 turns the watchdog off");
    }

    #[test]
    fn next_check_of_an_awaiting_session_is_the_end_of_the_await() {
        let f = Facts { idle_ms: 0, awaiting: true, ..quiet() };
        assert_eq!(next_check_ms(&f, TH, Some(5_000), None), Some(5_020));
        assert_eq!(next_check_ms(&f, TH, None, None), None);
    }

    #[test]
    fn the_stop_reminder_fires_once_per_episode_and_is_rate_limited() {
        use std::time::Duration;
        let t = Turns::default();
        let t0 = Instant::now();
        let s = |d: u64| t0 + Duration::from_secs(d);
        let never = u64::MAX;
        assert!(!t.stop(1, t0, never, never, false), "not due: nothing");
        assert!(t.stop(1, t0, never, never, true), "first turn end: reminded");
        assert_eq!(t.nudged_ms(1, s(5), never, never), Some(5_000));
        // the reminder's own turn: its working hint is not new work, and the stop that follows is not reminded again
        t.working(1, s(2));
        assert!(!t.stop(1, s(3), never, never, true));
        assert!(!t.stop(1, s(20 * 60), never, never, true), "however long after: same episode, no new work");
        assert!(t.nudged_ms(1, s(20 * 60), never, never).is_some());
        // a working hint outside the reminder's turn is real work: a new episode
        t.working(1, s(20 * 60 + 1));
        assert_eq!(t.nudged_ms(1, s(20 * 60 + 2), never, never), None);
        assert!(t.stop(1, s(20 * 60 + 2), never, never, true));
        // new work again soon after (a message 1 s ago), but the rate limit holds for 10 min
        assert!(!t.stop(1, s(20 * 60 + 60), never, 1_000, true));
        assert!(t.stop(1, s(31 * 60), never, 1_000, true), "after 10 min and new work: reminded again");
        // the user typing is new work too
        let u = Turns::default();
        assert!(u.stop(2, t0, never, never, true));
        assert!(!u.stop(2, s(11 * 60), never, never, true));
        assert!(u.stop(2, s(11 * 60), 1_000, never, true));
    }

    #[test]
    fn a_stall_episode_is_told_once_and_ends_only_on_real_work() {
        use std::time::Duration;
        let t = Turns::default();
        let t0 = Instant::now();
        let s = |d: u64| t0 + Duration::from_secs(d);
        let never = u64::MAX;
        assert!(!t.in_episode(1, t0, never, never));
        assert!(t.begin_stall(1, t0, never, never), "first look: tell");
        assert!(t.in_episode(1, s(10), never, never));
        // unchanged (a stray output byte does not count): never told again
        for d in [10, 600, 3600] {
            assert!(!t.begin_stall(1, s(d), never, never));
        }
        // a message arrived 5 s ago: the episode is over, the next stall is a new one
        assert!(!t.in_episode(1, s(4000), never, 5_000));
        assert!(t.begin_stall(1, s(4000), never, 5_000));
        assert!(!t.begin_stall(1, s(4100), never, 105_000));
        // the user typed
        assert!(t.begin_stall(1, s(5000), 1_000, 1_000_000));
        // a working hint
        t.working(1, s(5001));
        assert!(t.begin_stall(1, s(5002), 1_000_000, 1_000_000));
        t.retain(&std::collections::HashSet::new());
        assert!(!t.in_episode(1, s(5003), never, never));
    }

    #[test]
    fn work_tells_start_up_processes_from_what_the_session_started() {
        use std::collections::HashSet;
        use std::time::{Duration, Instant};
        let w = Work::default();
        let t0 = Instant::now();
        let set = |v: &[u32]| v.iter().copied().collect::<HashSet<u32>>();
        // the agent starts its MCP servers: part of the start-up
        assert!(!w.observe(1, t0, &set(&[10, 11])));
        assert!(!w.observe(1, t0 + Duration::from_secs(30), &set(&[10, 11, 12])));
        assert!(!w.busy(1));
        // later a new process appears (a background shell, a build, a lock wait)
        assert!(w.observe(1, t0 + Duration::from_secs(120), &set(&[10, 11, 12, 40])));
        assert!(w.busy(1));
        // it ends: the session is quiet again
        assert!(!w.observe(1, t0 + Duration::from_secs(200), &set(&[10, 11, 12])));
        assert!(!w.busy(2));
        w.retain(&HashSet::new());
        assert!(!w.busy(1));
    }

    #[test]
    fn a_slot_wait_holds_only_while_no_slot_is_free() {
        let w = Work::default();
        assert!(!w.slot_waiting(1, || true), "never asked");
        w.want_slot(1);
        assert!(w.slot_waiting(1, || true));
        assert!(!w.slot_waiting(1, || false), "a slot is free: it is not waiting any more");
        w.got_slot(1);
        assert!(!w.slot_waiting(1, || true));
    }

    #[test]
    fn setting_defaults_and_bounds() {
        assert_eq!(minutes(&json!({})), 15);
        assert_eq!(minutes(&json!({"stall_minutes": 0})), 0);
        assert_eq!(minutes(&json!({"stall_minutes": 12})), 12);
        assert_eq!(minutes(&json!({"stall_minutes": 999999})), MAX_MINUTES);
        assert_eq!(minutes(&json!({"stall_minutes": "x"})), 15);
        assert_eq!(display_minutes(0), 0);
        assert_eq!(display_minutes(1000), 1);
        assert_eq!(display_minutes(300_000), 5);
        assert_eq!(validate(&json!(7)), Some(7));
        assert_eq!(validate(&json!(-1)), None);
        assert_eq!(validate(&json!(MAX_MINUTES + 1)), None);
    }

    #[test]
    fn last_line_skips_blanks_and_clips() {
        let l = vec!["a".to_string(), "  continuo parado  ".to_string(), "".to_string()];
        assert_eq!(last_line(&l), "continuo parado");
        assert_eq!(last_line(&["x".repeat(500)]).chars().count(), 200);
        assert_eq!(last_line(&[]), "");
        // a status bar or a bare prompt says nothing
        let l = vec!["I will wait for CI to finish".to_string(), "⏵⏵ auto mode on".to_string(), ">".to_string()];
        assert_eq!(last_line(&l), "I will wait for CI to finish");
    }

    #[test]
    fn parent_text_is_one_short_line() {
        let t = parent_text("Tarefa 2", 7, 15, Why::Nudged);
        assert_eq!(t, "\"Tarefa 2\" (session 7) stopped without done/await/ask and ignored one reminder. Check it: axelstra read 7");
        let t = parent_text("x", 7, 15, Why::Quiet);
        assert!(t.contains("quiet 15 min") && !t.contains('\n'), "{t}");
        let t = parent_text("x", 7, 15, Why::NeverStarted);
        assert!(t.contains("never reported a turn") && t.contains("axelstra read 7"), "{t}");
    }

    #[test]
    fn several_children_make_one_line() {
        let t = parent_text_many(&[("Tarefa 1".into(), 3), ("Tarefa 2".into(), 4)]);
        assert!(t.starts_with("2 sessions") && t.contains("\"Tarefa 1\" (session 3)") && t.contains("\"Tarefa 2\" (session 4)"));
        assert!(!t.contains('\n'));
    }
}
