//! "Waiting for an answer" is never read off the terminal text: it is something the agent says. An agent says it with
//! `axelstra ask` / `axelstra escalate` (the message bus keeps those) or through its own hook (`axelstra hook ...`
//! sends `session.hint` with a kind and the text). Without such an event a session is only working or idle.

/// Rule typed into every lead/worker prompt (English on purpose: agents are prompted in English). Writing "waiting for
/// your decision" in the terminal tells nobody; the stall watchdog only flags the silence after the fact.
pub const NEVER_WAIT_RULE: &str = "Never wait silently for the human: if you need a decision, a confirmation or a permission, or you are blocked, run `axelstra ask $AXELSTRA_PARENT \"<question>\"` (it blocks until answered) or `axelstra escalate \"<problem>\"`; if the choice is reversible, decide, say so in your report and carry on. Text such as \"waiting for your decision\" in the terminal reaches nobody, and after a few quiet minutes you are flagged as stalled.";

/// What an agent's own hook (Claude Code's Notification, Stop, UserPromptSubmit) said it is doing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum HintState {
    Waiting,
    Idle,
    Working,
}

impl HintState {
    pub fn parse(s: &str) -> Option<HintState> {
        match s {
            "waiting" => Some(HintState::Waiting),
            "idle" => Some(HintState::Idle),
            "working" => Some(HintState::Working),
            _ => None,
        }
    }
}

/// A hint older than this no longer counts: a missed Stop must not leave a red dot forever.
pub const HINT_TTL_MS: u64 = 2 * 60 * 60 * 1000;
/// Output within this long after the hint is the screen finishing its redraw, not the agent working again.
const HINT_SLACK_MS: u64 = 1500;

/// True while the latest hint says waiting and nothing happened since: no typing, no new output, not expired.
/// `age_ms` is the time since the hint, `since_input_ms` since the user last typed, `idle_ms` since the last output.
pub fn hint_waiting(state: HintState, age_ms: u64, since_input_ms: u64, idle_ms: u64) -> bool {
    state == HintState::Waiting && age_ms < HINT_TTL_MS && since_input_ms > age_ms && idle_ms + HINT_SLACK_MS >= age_ms
}

/// Why an agent needs the user: a permission prompt, a question (an elicitation, `axelstra ask`) or, for a session
/// that only sits at its input box, `IdleInput`, which is not a wait.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum WaitKind {
    Permission,
    Question,
    IdleInput,
}

impl WaitKind {
    pub fn parse(s: &str) -> Option<WaitKind> {
        match s {
            "permission" => Some(WaitKind::Permission),
            "question" => Some(WaitKind::Question),
            "idle_input" => Some(WaitKind::IdleInput),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            WaitKind::Permission => "permission",
            WaitKind::Question => "question",
            WaitKind::IdleInput => "idle_input",
        }
    }
}

/// What the agent said it waits for: the kind and its own words.
#[derive(Clone, Debug, PartialEq)]
pub struct Wait {
    pub kind: WaitKind,
    pub text: String,
}

/// Longest event text kept (characters).
pub const WAIT_TEXT_MAX: usize = 500;

/// The text an agent sent, safe to store and show: no control characters, trimmed, at most `WAIT_TEXT_MAX` characters.
pub fn clean_text(text: &str) -> String {
    let t: String = text.chars().filter(|c| *c == '\n' || !c.is_control()).collect();
    let t = t.trim();
    match t.char_indices().nth(WAIT_TEXT_MAX) {
        Some((i, _)) => format!("{}…", t[..i].trim_end()),
        None => t.to_string(),
    }
}

struct Hint {
    at: std::time::Instant,
    state: HintState,
    wait: Option<Wait>,
    /// the parent was told about this wait
    told: bool,
}

/// Latest hint per session, with the moment it arrived.
#[derive(Default)]
pub struct Hints(std::sync::Mutex<std::collections::HashMap<u32, Hint>>);

impl Hints {
    /// Records the agent's latest event. Only `Waiting` with a kind that needs the user counts as a wait: an
    /// `idle_input` (Claude's `idle_prompt`: it finished and sits at the prompt) is just idle.
    pub fn set(&self, id: u32, state: HintState, wait: Option<Wait>) {
        let wait = wait.filter(|w| state == HintState::Waiting && w.kind != WaitKind::IdleInput);
        let state = if state == HintState::Waiting && wait.is_none() { HintState::Idle } else { state };
        if let Ok(mut m) = self.0.lock() {
            // the agent repeating the wait it is already in (a Notification fires again) is the same wait
            let told = state == HintState::Waiting && m.get(&id).is_some_and(|h| h.state == HintState::Waiting && h.told && h.wait == wait);
            m.insert(id, Hint { at: std::time::Instant::now(), state, wait, told });
        }
    }

    pub fn clear(&self, id: u32) {
        if let Ok(mut m) = self.0.lock() {
            m.remove(&id);
        }
    }

    /// What the session's hint says it waits for, if it still does, given how long it has been quiet (see `hint_waiting`).
    pub fn waiting(&self, id: u32, since_input_ms: u64, idle_ms: u64) -> Option<Wait> {
        let m = self.0.lock().ok()?;
        let h = m.get(&id)?;
        hint_waiting(h.state, h.at.elapsed().as_millis() as u64, since_input_ms, idle_ms).then(|| h.wait.clone()).flatten()
    }

    /// The wait to tell the parent about, once per wait: a second call for the same event gives nothing.
    pub fn take_untold(&self, id: u32) -> Option<Wait> {
        let mut m = self.0.lock().ok()?;
        let h = m.get_mut(&id)?;
        if h.told || h.state != HintState::Waiting {
            return None;
        }
        h.told = true;
        h.wait.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(kind: WaitKind, text: &str) -> Option<Wait> {
        Some(Wait { kind, text: text.into() })
    }

    #[test]
    fn hint_waits_until_input_output_or_expiry() {
        use HintState::*;
        // hint 10 s ago, no typing ever, quiet since: waiting
        assert!(hint_waiting(Waiting, 10_000, u64::MAX, 10_000));
        // output right after the hint is the redraw
        assert!(hint_waiting(Waiting, 10_000, u64::MAX, 9_000));
        // real output later: working again
        assert!(!hint_waiting(Waiting, 10_000, u64::MAX, 2_000));
        // user typed after the hint
        assert!(!hint_waiting(Waiting, 10_000, 3_000, 10_000));
        // typed before the hint: still waiting
        assert!(hint_waiting(Waiting, 10_000, 20_000, 10_000));
        // expired
        assert!(!hint_waiting(Waiting, HINT_TTL_MS, u64::MAX, HINT_TTL_MS));
        assert!(!hint_waiting(Idle, 10_000, u64::MAX, 10_000));
        assert!(!hint_waiting(Working, 10_000, u64::MAX, 10_000));
    }

    #[test]
    fn hints_replace_and_clear() {
        let h = Hints::default();
        h.set(1, HintState::Waiting, w(WaitKind::Permission, "Allow Bash?"));
        std::thread::sleep(std::time::Duration::from_millis(5));
        assert_eq!(h.waiting(1, u64::MAX, 60_000), w(WaitKind::Permission, "Allow Bash?"));
        assert!(h.waiting(2, u64::MAX, 60_000).is_none());
        h.set(1, HintState::Working, None);
        assert!(h.waiting(1, u64::MAX, 60_000).is_none());
        h.set(1, HintState::Waiting, w(WaitKind::Question, "q"));
        h.clear(1);
        assert!(h.waiting(1, u64::MAX, 60_000).is_none());
        assert_eq!(HintState::parse("idle"), Some(HintState::Idle));
        assert_eq!(HintState::parse("x"), None);
    }

    #[test]
    fn a_wait_needs_a_kind_and_idle_input_is_not_one() {
        let h = Hints::default();
        // the old hook said only "waiting": no event kind, no wait
        h.set(1, HintState::Waiting, None);
        h.set(2, HintState::Waiting, w(WaitKind::IdleInput, "Claude is waiting for your input"));
        std::thread::sleep(std::time::Duration::from_millis(5));
        assert!(h.waiting(1, u64::MAX, 60_000).is_none());
        assert!(h.waiting(2, u64::MAX, 60_000).is_none());
        assert!(h.take_untold(2).is_none());
    }

    #[test]
    fn the_parent_is_told_once_per_wait() {
        let h = Hints::default();
        h.set(1, HintState::Waiting, w(WaitKind::Permission, "p"));
        assert_eq!(h.take_untold(1), w(WaitKind::Permission, "p"));
        assert!(h.take_untold(1).is_none());
        // the same wait repeated is the same wait
        h.set(1, HintState::Waiting, w(WaitKind::Permission, "p"));
        assert!(h.take_untold(1).is_none());
        // a different one is a new wait
        h.set(1, HintState::Waiting, w(WaitKind::Question, "q"));
        assert_eq!(h.take_untold(1), w(WaitKind::Question, "q"));
        h.set(1, HintState::Idle, None);
        assert!(h.take_untold(1).is_none());
    }

    #[test]
    fn kinds_round_trip_and_text_is_cleaned() {
        for k in [WaitKind::Permission, WaitKind::Question, WaitKind::IdleInput] {
            assert_eq!(WaitKind::parse(k.as_str()), Some(k));
        }
        assert_eq!(WaitKind::parse("screen"), None);
        assert_eq!(clean_text("  a[31m b
"), "a[31m b");
        assert_eq!(clean_text(&"x".repeat(900)).chars().count(), WAIT_TEXT_MAX + 1);
    }

    /// Text on the screen never makes a wait: there is no screen input to `Hints` at all, and a finished session or
    /// the user's own question mark leaves it idle.
    #[test]
    fn a_session_without_events_never_waits() {
        let h = Hints::default();
        assert!(h.waiting(7, u64::MAX, 60_000).is_none());
        assert!(h.take_untold(7).is_none());
    }
}
