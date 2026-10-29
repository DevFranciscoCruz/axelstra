//! Inactivity watchdog: an agent that went quiet without reporting (`done`/`escalate`) is "stalled", not idle.
//! The decision is a pure function of facts the registry gathers (`Facts`), so it is tested without sessions.

use serde_json::Value;

/// Minutes of silence before an agent counts as stalled, unless the settings say otherwise (`stall_minutes`, 0 = off).
pub const DEFAULT_MINUTES: u64 = 5;
/// Upper bound accepted for the setting (a day).
pub const MAX_MINUTES: u64 = 1440;

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
    /// it said (`axelstra await`) that it only waits for something external, within its limit
    pub awaiting: bool,
    /// a child session of it is still running (it legitimately waits for it)
    pub live_children: bool,
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
    /// its parent is a live agent session: that parent's `idle_ms` (`None`: no parent, a terminal, or it exited)
    pub parent_idle_ms: Option<u64>,
    /// ms since the watchdog told the parent about this stall (`None`: not told)
    pub notified_ms: Option<u64>,
}

impl Facts {
    /// ms of silence that count towards the threshold: no output, no typing and no message.
    pub fn quiet_ms(&self) -> u64 {
        self.idle_ms.min(self.since_input_ms).min(self.since_msg_ms)
    }
}

/// True when the agent has been quiet (no output, no typing, no message) for `threshold_ms` with nothing that explains
/// it (0 = off).
pub fn is_stalled(f: &Facts, threshold_ms: u64) -> bool {
    threshold_ms > 0
        && f.agent
        && !f.main
        && f.running
        && !f.reported
        && !f.escalated
        && !f.awaiting
        && !f.live_children
        && !f.bg_work
        && !f.slot_wait
        && f.quiet_ms() >= threshold_ms
}

/// True when the stall needs a person: the session is stalled and nobody handles it. A parent agent that was told
/// less than a threshold ago, or that is still working, handles it; with no live agent parent, or a parent that stayed
/// idle for a whole threshold after being told, the user is the only one left.
pub fn needs_human(f: &Facts, threshold_ms: u64) -> bool {
    is_stalled(f, threshold_ms)
        && match f.parent_idle_ms {
            None => true,
            Some(pidle) => f.notified_ms.is_some_and(|a| a >= threshold_ms) && pidle >= threshold_ms,
        }
}

/// Recheck period while a background process of the tree explains the silence: no OS event says a grandchild ended.
pub const BG_RECHECK_MS: u64 = 60_000;

/// When the human alert of a stalled session with a parent agent comes due, in ms: a threshold after the parent was
/// told, and a threshold after the parent last did anything. `None`: nothing time alone changes.
pub fn next_human_ms(f: &Facts, threshold_ms: u64) -> Option<u64> {
    let (told, pidle) = (f.notified_ms?, f.parent_idle_ms?);
    if !is_stalled(f, threshold_ms) {
        return None;
    }
    let left = threshold_ms.saturating_sub(told).max(threshold_ms.saturating_sub(pidle));
    (left > 0).then_some(left + 20)
}

/// When to look at this session again, in ms, if nothing happens meanwhile: the moment its quiet spell reaches the
/// threshold, or (while it awaits something) the moment the await expires. `None`: nothing time alone can change, an
/// event (pulse) will say. This is what lets the watchdog sleep instead of ticking.
pub fn next_check_ms(f: &Facts, threshold_ms: u64, await_left_ms: Option<u64>) -> Option<u64> {
    if threshold_ms == 0 || !f.agent || f.main || !f.running || f.reported || f.escalated || f.live_children {
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
        return f.bg_work.then_some(BG_RECHECK_MS);
    }
    Some(threshold_ms - quiet + 20)
}

/// What the stall message says was ruled out (the watchdog only speaks when none of it applies).
const RULED_OUT: &str = "no output, no typing, no message, no background process, no `axelstra await`, no live children";

/// The English text typed into the parent's session (agent-facing, so it stays English).
pub fn parent_text(label: &str, id: u32, minutes: u64, last_line: &str) -> String {
    let line = if last_line.is_empty() { String::new() } else { format!(" Last screen line: {last_line}.") };
    format!(
        "\"{label}\" (session {id}) has been stalled for {minutes} min ({RULED_OUT}). Probably it ended its turn without \
         calling `axelstra done` or `axelstra escalate`, or it waits for a decision.{line} Check it with `axelstra read {id}` \
         or nudge it with `axelstra msg {id} \"<text>\"`."
    )
}

/// One message for several sessions of the same parent that stalled in the same pass: `(label, id)` each.
pub fn parent_text_many(items: &[(String, u32)], minutes: u64) -> String {
    let list: Vec<String> = items.iter().map(|(l, id)| format!("\"{l}\" (session {id})")).collect();
    format!(
        "{} sessions have been stalled for {minutes} min ({RULED_OUT}): {}. Probably they ended their turn without calling \
         `axelstra done`. Check each with `axelstra read <session>` or nudge it with `axelstra msg <session> \"<text>\"`.",
        items.len(),
        list.join(", ")
    )
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

    fn quiet() -> Facts {
        Facts { agent: true, running: true, idle_ms: 6 * 60_000, since_input_ms: u64::MAX, since_msg_ms: u64::MAX, ..Facts::default() }
    }

    #[test]
    fn fires_after_the_threshold_only() {
        assert!(is_stalled(&quiet(), 300_000));
        assert!(!is_stalled(&Facts { idle_ms: 4 * 60_000, ..quiet() }, 300_000));
        assert!(is_stalled(&Facts { idle_ms: 4 * 60_000, ..quiet() }, 180_000));
    }

    #[test]
    fn typing_counts_as_activity() {
        assert!(!is_stalled(&Facts { since_input_ms: 10_000, ..quiet() }, 300_000));
    }

    #[test]
    fn does_not_fire_when_something_explains_the_silence() {
        for f in [
            Facts { reported: true, ..quiet() },
            Facts { escalated: true, ..quiet() },
            Facts { awaiting: true, ..quiet() },
            Facts { live_children: true, ..quiet() },
            // legit work no screen shows: a background shell / build / lock wait, a wait for a worker slot, a message
            Facts { bg_work: true, ..quiet() },
            Facts { slot_wait: true, ..quiet() },
            Facts { since_msg_ms: 10_000, ..quiet() },
            Facts { running: false, ..quiet() },
            Facts { main: true, ..quiet() },
            Facts { agent: false, ..quiet() },
        ] {
            assert!(!is_stalled(&f, 300_000), "{f:?}");
        }
        assert!(!is_stalled(&quiet(), 0), "0 turns the watchdog off");
    }

    #[test]
    fn next_check_is_the_moment_the_quiet_spell_reaches_the_threshold() {
        assert_eq!(next_check_ms(&Facts { idle_ms: 400_000, ..quiet() }, 300_000, None), None, "already stalled: nothing to wait for");
        let f = Facts { idle_ms: 100_000, since_input_ms: 250_000, ..quiet() };
        assert_eq!(next_check_ms(&f, 300_000, None), Some(200_020));
        // typing moves it
        assert_eq!(next_check_ms(&Facts { since_input_ms: 10_000, ..f }, 300_000, None), Some(290_020));
        assert_eq!(next_check_ms(&Facts { idle_ms: 0, ..quiet() }, 300_000, None), Some(300_020));
    }

    #[test]
    fn a_background_process_is_rechecked_by_a_bounded_deadline_a_slot_wait_by_event() {
        let f = Facts { bg_work: true, ..quiet() };
        assert_eq!(next_check_ms(&f, 300_000, None), Some(BG_RECHECK_MS));
        assert_eq!(next_check_ms(&Facts { slot_wait: true, ..quiet() }, 300_000, None), None);
        // a message restarts the quiet spell
        assert_eq!(next_check_ms(&Facts { since_msg_ms: 100_000, ..quiet() }, 300_000, None), Some(200_020));
    }

    fn child(parent_idle: Option<u64>, notified: Option<u64>) -> Facts {
        Facts { parent_idle_ms: parent_idle, notified_ms: notified, ..quiet() }
    }

    #[test]
    fn the_user_is_alerted_only_when_nobody_handles_the_stall() {
        let th = 300_000;
        // no live agent parent: the user is the only one
        assert!(needs_human(&child(None, None), th));
        // a parent agent that was not told yet, or was told a moment ago, handles it
        assert!(!needs_human(&child(Some(10 * 60_000), None), th));
        assert!(!needs_human(&child(Some(10 * 60_000), Some(60_000)), th));
        // told long ago but the parent is working on it
        assert!(!needs_human(&child(Some(5_000), Some(400_000)), th));
        // told long ago and the parent did nothing for a whole threshold
        assert!(needs_human(&child(Some(400_000), Some(400_000)), th));
        // not stalled at all: never
        assert!(!needs_human(&Facts { bg_work: true, ..child(None, None) }, th));
    }

    #[test]
    fn the_human_deadline_is_one_computed_moment() {
        let th = 300_000;
        assert_eq!(next_human_ms(&child(None, None), th), None, "no parent: no deadline, it is due now");
        assert_eq!(next_human_ms(&child(Some(10 * 60_000), None), th), None, "not told: the telling is an event");
        assert_eq!(next_human_ms(&child(Some(400_000), Some(100_000)), th), Some(200_020));
        // the parent was active a minute ago: due when it has been idle a threshold
        assert_eq!(next_human_ms(&child(Some(60_000), Some(400_000)), th), Some(240_020));
        assert_eq!(next_human_ms(&child(Some(400_000), Some(400_000)), th), None, "already due");
        assert_eq!(next_human_ms(&Facts { awaiting: true, ..child(Some(0), Some(0)) }, th), None);
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
    fn next_check_ignores_what_only_an_event_can_change() {
        let f = Facts { idle_ms: 0, ..quiet() };
        for g in [
            Facts { reported: true, ..f },
            Facts { escalated: true, ..f },
            Facts { live_children: true, ..f },
            Facts { running: false, ..f },
            Facts { main: true, ..f },
            Facts { agent: false, ..f },
        ] {
            assert_eq!(next_check_ms(&g, 300_000, None), None, "{g:?}");
        }
        assert_eq!(next_check_ms(&f, 0, None), None, "0 turns the watchdog off");
    }

    #[test]
    fn next_check_of_an_awaiting_session_is_the_end_of_the_await() {
        let f = Facts { idle_ms: 0, awaiting: true, ..quiet() };
        assert_eq!(next_check_ms(&f, 300_000, Some(5_000)), Some(5_020));
        assert_eq!(next_check_ms(&f, 300_000, None), None);
    }

    #[test]
    fn setting_defaults_and_bounds() {
        assert_eq!(minutes(&json!({})), 5);
        assert_eq!(minutes(&json!({"stall_minutes": 0})), 0);
        assert_eq!(minutes(&json!({"stall_minutes": 12})), 12);
        assert_eq!(minutes(&json!({"stall_minutes": 999999})), MAX_MINUTES);
        assert_eq!(minutes(&json!({"stall_minutes": "x"})), 5);
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
    fn parent_text_names_the_session_and_the_line() {
        let t = parent_text("Tarefa 2", 7, 5, "waiting for you");
        assert!(t.contains("Tarefa 2") && t.contains("session 7") && t.contains("5 min") && t.contains("waiting for you"));
        assert!(t.contains("axelstra msg 7") && t.contains("Probably"));
        assert!(!parent_text("x", 7, 5, "").contains("Last screen line"));
    }

    #[test]
    fn several_children_make_one_message() {
        let t = parent_text_many(&[("Tarefa 1".into(), 3), ("Tarefa 2".into(), 4)], 5);
        assert!(t.starts_with("2 sessions") && t.contains("\"Tarefa 1\" (session 3)") && t.contains("\"Tarefa 2\" (session 4)"));
    }
}
