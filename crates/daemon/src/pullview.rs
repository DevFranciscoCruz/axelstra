//! What a coordinator (a lead or the project main) PULLS about its sessions, now that the daemon pushes only a child's
//! `done` and the pendencies that need someone to act: per `session.list` row, since when it is quiet, whether it shows
//! signs of life and the one thing its parent should do next. And which bus kinds `msg.wait` (`axelstra inbox --wait`)
//! blocks on. Pure functions over facts the server gathers; no clock, no lock.
//!
//! Silence alone never makes a session dead here: `liveness.agent == false` only says the agent showed nothing in
//! this turn, never that it is gone (`pty` says whether the process is alive).

use serde_json::{json, Value};

/// Output, typing or a hook hint within this long counts as working (the same window hibernation calls quiet).
pub const WORKING_MS: u64 = 20_000;

/// Unix ms now (what `quiet_since` is dated against).
pub fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// What the server knows about one session row.
#[derive(Clone, Debug, Default)]
pub struct Facts {
    pub id: u32,
    /// the process is alive (a sleeper's is not)
    pub pty: bool,
    pub hibernated: bool,
    pub paused: bool,
    /// it said `axelstra await` (CI, a deploy) and that still holds
    pub awaiting: bool,
    /// a child of it still runs and has not reported
    pub live_children: bool,
    /// it reported `done`
    pub finished: bool,
    /// its `done` report is still unread in its parent's inbox
    pub done_unread: bool,
    /// the `ask`/`escalate` it waits an answer for
    pub pending_msg: Option<u64>,
    /// its own hook says it waits on a permission (`Some(true)`) or a question (`Some(false)`)
    pub hook_wait: Option<bool>,
    /// a real stall: the watchdog told its parent, or the user is told
    pub stalled: bool,
    /// the latest hook hint said working, and how long ago (ms); `None`: no hint or another state
    pub hint_working_ms: Option<u64>,
    /// ms since the latest hook hint of any kind (`None`: the agent never sent one)
    pub hint_age_ms: Option<u64>,
    /// a build, a shell or `axelstra lock` under it is alive
    pub bg_work: bool,
    pub idle_ms: u64,
    pub since_input_ms: u64,
    pub since_msg_ms: u64,
}

impl Facts {
    /// ms since the last output, typing or message.
    pub fn quiet_ms(&self) -> u64 {
        self.idle_ms.min(self.since_input_ms).min(self.since_msg_ms)
    }

    /// ms since the current turn began: the last typing or message (`u64::MAX`: it was never given one).
    fn turn_ms(&self) -> u64 {
        self.since_input_ms.min(self.since_msg_ms)
    }

    fn working(&self) -> bool {
        self.pty && (self.hint_working_ms.is_some() || self.bg_work || self.quiet_ms() < WORKING_MS)
    }
}

/// Unix ms of the last output, typing or message; `None` while it works (or it never did anything we can date).
pub fn quiet_since(f: &Facts, now_ms: u64) -> Option<u64> {
    if f.working() {
        return None;
    }
    let q = f.quiet_ms();
    (q != u64::MAX).then(|| now_ms.saturating_sub(q))
}

/// `{pty, agent}`: the process is alive; the agent itself showed life in this turn (a hook hint or output since the
/// last typing or message, layered: either is enough). False `agent` is no verdict of death.
pub fn liveness(f: &Facts) -> Value {
    let turn = f.turn_ms();
    let hinted = f.hint_age_ms.is_some_and(|a| a < turn);
    let printed = f.idle_ms < turn;
    json!({"pty": f.pty, "agent": f.pty && (hinted || printed)})
}

/// The one thing the parent should do next, as a literal hint: `none` (keep waiting), `reply <msg-id>`, `answer
/// permission in session N`, `answer question in session N`, `collect done`, `read N` (a real stall). Ordered: a
/// question blocks its asker, then an uncollected report, then what explains a wait, then a stall.
pub fn next_action(f: &Facts) -> String {
    if let Some(m) = f.pending_msg {
        return format!("reply {m}");
    }
    if f.pty && !f.hibernated {
        match f.hook_wait {
            Some(true) => return format!("answer permission in session {}", f.id),
            Some(false) => return format!("answer question in session {}", f.id),
            None => {}
        }
    }
    if f.finished {
        return if f.done_unread { "collect done".into() } else { "none".into() };
    }
    if !f.pty || f.hibernated || f.paused || f.awaiting || f.live_children || f.working() {
        return "none".into();
    }
    if f.stalled {
        return format!("read {}", f.id);
    }
    "none".into()
}

/// The bus kinds behind the `--types` names of `axelstra inbox --wait`: `done`, `question` (ask), `escalation`
/// (escalate), `wait` (a child's permission/input wait or stall, sent as a `notice`), and the bus kinds by their own
/// names (`message`, `reply`, `notice`, `ask`, `escalate`). Empty: the default, every pushed kind.
pub fn wait_kinds(types: &[String]) -> Result<Vec<&'static str>, String> {
    const DEFAULT: &[&str] = &["done", "ask", "escalate", "notice"];
    if types.is_empty() {
        return Ok(DEFAULT.to_vec());
    }
    let mut out: Vec<&'static str> = vec![];
    for t in types {
        let k = match t.trim() {
            "done" => "done",
            "question" | "ask" => "ask",
            "escalation" | "escalate" => "escalate",
            "wait" | "notice" => "notice",
            "message" => "message",
            "reply" => "reply",
            other => return Err(other.to_string()),
        };
        if !out.contains(&k) {
            out.push(k);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: u64 = 60_000;

    fn quiet(id: u32) -> Facts {
        Facts { id, pty: true, idle_ms: 10 * MIN, since_input_ms: u64::MAX, since_msg_ms: 12 * MIN, ..Default::default() }
    }

    #[test]
    fn a_quiet_session_with_nothing_pending_needs_nothing() {
        assert_eq!(next_action(&quiet(3)), "none");
    }

    #[test]
    fn a_pending_question_asks_for_a_reply_to_that_message() {
        let f = Facts { pending_msg: Some(41), stalled: true, ..quiet(3) };
        assert_eq!(next_action(&f), "reply 41");
    }

    #[test]
    fn a_hook_wait_names_the_session_to_answer() {
        assert_eq!(next_action(&Facts { hook_wait: Some(true), ..quiet(7) }), "answer permission in session 7");
        assert_eq!(next_action(&Facts { hook_wait: Some(false), ..quiet(7) }), "answer question in session 7");
        // a sleeper has no screen to answer on
        assert_eq!(next_action(&Facts { hook_wait: Some(true), hibernated: true, ..quiet(7) }), "none");
    }

    #[test]
    fn a_report_is_collected_once() {
        assert_eq!(next_action(&Facts { finished: true, done_unread: true, ..quiet(3) }), "collect done");
        assert_eq!(next_action(&Facts { finished: true, done_unread: false, stalled: true, ..quiet(3) }), "none");
        // a sleeper that reported still has its report to collect
        assert_eq!(next_action(&Facts { finished: true, done_unread: true, hibernated: true, pty: false, ..quiet(3) }), "collect done");
    }

    #[test]
    fn explained_waits_are_left_alone_even_when_stalled_is_set() {
        for f in [
            Facts { awaiting: true, stalled: true, ..quiet(3) },
            Facts { live_children: true, stalled: true, ..quiet(3) },
            Facts { hibernated: true, pty: false, stalled: true, ..quiet(3) },
            Facts { paused: true, stalled: true, ..quiet(3) },
            Facts { bg_work: true, stalled: true, ..quiet(3) },
            Facts { hint_working_ms: Some(5 * MIN), stalled: true, ..quiet(3) },
            Facts { idle_ms: 1000, stalled: true, ..quiet(3) },
        ] {
            assert_eq!(next_action(&f), "none", "{f:?}");
        }
    }

    #[test]
    fn a_real_stall_says_read_the_session() {
        assert_eq!(next_action(&Facts { stalled: true, ..quiet(9) }), "read 9");
    }

    #[test]
    fn quiet_since_is_null_while_working_and_dates_the_last_sign_otherwise() {
        let now = 100 * MIN;
        assert_eq!(quiet_since(&quiet(1), now), Some(90 * MIN));
        assert_eq!(quiet_since(&Facts { idle_ms: 1000, ..quiet(1) }, now), None);
        assert_eq!(quiet_since(&Facts { hint_working_ms: Some(30 * MIN), ..quiet(1) }, now), None);
        assert_eq!(quiet_since(&Facts { bg_work: true, ..quiet(1) }, now), None);
        // the latest of output, typing and message counts
        assert_eq!(quiet_since(&Facts { since_input_ms: 2 * MIN, ..quiet(1) }, now), Some(98 * MIN));
        // a dead process is not working, whatever its last hint said
        assert_eq!(quiet_since(&Facts { pty: false, hint_working_ms: Some(1), ..quiet(1) }, now), Some(90 * MIN));
    }

    #[test]
    fn liveness_is_layered_and_silence_is_no_death() {
        // output after the last message: the agent is alive in this turn
        let f = Facts { idle_ms: 5 * MIN, since_msg_ms: 6 * MIN, ..quiet(1) };
        assert_eq!(liveness(&f), json!({"pty": true, "agent": true}));
        // nothing since the last message, but a hook hint came after it
        let f = Facts { idle_ms: 9 * MIN, since_msg_ms: 6 * MIN, hint_age_ms: Some(MIN), ..quiet(1) };
        assert_eq!(liveness(&f), json!({"pty": true, "agent": true}));
        // silence since the message: no sign, but the process is still alive and nothing says dead
        let f = Facts { idle_ms: 9 * MIN, since_msg_ms: 6 * MIN, hint_age_ms: Some(8 * MIN), ..quiet(1) };
        assert_eq!(liveness(&f), json!({"pty": true, "agent": false}));
        // never given a turn: any output counts
        let f = Facts { since_msg_ms: u64::MAX, ..quiet(1) };
        assert_eq!(liveness(&f)["agent"], true);
        assert_eq!(liveness(&Facts { pty: false, ..quiet(1) }), json!({"pty": false, "agent": false}));
    }

    #[test]
    fn wait_types_map_to_bus_kinds() {
        assert_eq!(wait_kinds(&[]).unwrap(), vec!["done", "ask", "escalate", "notice"]);
        let t: Vec<String> = ["done", "question", "escalation", "question"].iter().map(|s| s.to_string()).collect();
        assert_eq!(wait_kinds(&t).unwrap(), vec!["done", "ask", "escalate"]);
        assert_eq!(wait_kinds(&["wait".into()]).unwrap(), vec!["notice"]);
        assert_eq!(wait_kinds(&["bogus".into()]), Err("bogus".into()));
    }
}
