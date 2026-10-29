//! "Waiting for an answer" is never read off the terminal text: it is something the agent says. An agent says it with
//! `axelstra ask` / `axelstra escalate` (the message bus keeps those) or through its own hook (`axelstra hook ...`
//! sends `session.hint` with a kind and the text). Without such an event a session is only working or idle.

/// Rule typed into every lead/worker prompt (English on purpose: agents are prompted in English). Writing "waiting for
/// your decision" in the terminal tells nobody; the stall watchdog only flags the silence after the fact.
pub const NEVER_WAIT_RULE: &str = "Never wait silently for the human: if you need a decision, a confirmation or a permission, or you are blocked, run `axelstra ask $AXELSTRA_PARENT \"<question>\"` (it blocks until answered) or `axelstra escalate \"<problem>\"`; if the choice is reversible, decide, say so in your report and carry on. Text such as \"waiting for your decision\" in the terminal reaches nobody, and after a few quiet minutes you are flagged as stalled. If you only wait for something external (a CI run, a deploy, a long process), run `axelstra await --reason \"CI #123\" [--for-min N]` before you stop instead of sitting silent: you are not flagged as stalled meanwhile, and it clears itself when you work again (`axelstra await --clear` ends it).";

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

    /// The latest state the agent's hook reported and how long ago (ms); `None` when it never reported (no hooks).
    pub fn state_of(&self, id: u32) -> Option<(HintState, u64)> {
        let m = self.0.lock().ok()?;
        let h = m.get(&id)?;
        Some((h.state, h.at.elapsed().as_millis() as u64))
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

/// Longest a session may stay "awaiting" something external; after that the stall watchdog speaks (once).
pub const AWAIT_CAP_MS: u64 = 2 * 60 * 60 * 1000;
/// Output within this long after `await` is the agent finishing its turn; later output means it works again.
pub const AWAIT_GRACE_MS: u64 = 30_000;
/// Longest reason kept (characters).
pub const AWAIT_REASON_MAX: usize = 200;

/// True while an `await` still holds: not expired (`limit_ms`), no typing and no real output since (see `AWAIT_GRACE_MS`).
/// `age_ms` is the time since `await`, `since_input_ms` since the user last typed, `idle_ms` since the last output.
pub fn await_active(age_ms: u64, limit_ms: u64, since_input_ms: u64, idle_ms: u64) -> bool {
    age_ms < limit_ms.min(AWAIT_CAP_MS) && since_input_ms > age_ms && idle_ms + AWAIT_GRACE_MS >= age_ms
}

/// The limit in ms for `--for-min` (None = the cap).
pub fn await_limit_ms(for_min: Option<u64>) -> u64 {
    for_min.map_or(AWAIT_CAP_MS, |m| m.saturating_mul(60_000).clamp(60_000, AWAIT_CAP_MS))
}

/// What `session.list` shows for an awaiting session.
#[derive(Clone, Debug, PartialEq)]
pub struct AwaitView {
    pub reason: String,
    /// unix ms when `await` was called
    pub since: u64,
    /// unix ms when it expires on its own
    pub until: u64,
}

struct Await {
    at: std::time::Instant,
    since: u64,
    reason: String,
    limit_ms: u64,
}

/// Sessions that said they only wait for something external (CI, a deploy), by session id.
#[derive(Default)]
pub struct Awaits(std::sync::Mutex<std::collections::HashMap<u32, Await>>);

impl Awaits {
    pub fn set(&self, id: u32, reason: &str, for_min: Option<u64>) {
        let since = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64);
        let reason: String = clean_text(reason).chars().take(AWAIT_REASON_MAX).collect();
        let a = Await { at: std::time::Instant::now(), since, reason, limit_ms: await_limit_ms(for_min) };
        if let Ok(mut m) = self.0.lock() {
            m.insert(id, a);
        }
    }

    /// True when there was one to clear.
    pub fn clear(&self, id: u32) -> bool {
        self.0.lock().ok().is_some_and(|mut m| m.remove(&id).is_some())
    }

    /// The await, if it still holds given how long the session has been quiet (see `await_active`).
    pub fn active(&self, id: u32, since_input_ms: u64, idle_ms: u64) -> Option<AwaitView> {
        let m = self.0.lock().ok()?;
        let a = m.get(&id)?;
        let age = a.at.elapsed().as_millis() as u64;
        await_active(age, a.limit_ms, since_input_ms, idle_ms).then(|| AwaitView {
            reason: a.reason.clone(),
            since: a.since,
            until: a.since + a.limit_ms.min(AWAIT_CAP_MS),
        })
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

    /// What clears a wait: the Stop hook (idle), UserPromptSubmit (working), the user typing, new output. An
    /// `idle_prompt` after a real wait must not leave the old one standing either.
    #[test]
    fn a_wait_is_cleared_by_stop_prompt_submit_input_and_output() {
        let wait = |h: &Hints| h.set(1, HintState::Waiting, w(WaitKind::Permission, "Allow Bash?"));
        let h = Hints::default();
        wait(&h);
        std::thread::sleep(std::time::Duration::from_millis(5));
        assert!(h.waiting(1, u64::MAX, 60_000).is_some());
        // (output after the redraw slack clears it too: see hint_waits_until_input_output_or_expiry)
        // the user typed after the hint
        assert!(h.waiting(1, 0, 60_000).is_none());
        for next in [HintState::Idle, HintState::Working] {
            wait(&h);
            h.set(1, next, None);
            assert!(h.waiting(1, u64::MAX, 60_000).is_none(), "{next:?}");
            assert!(h.take_untold(1).is_none(), "{next:?}");
        }
        // an idle_prompt after a real wait replaces it (the agent is at its input box now)
        wait(&h);
        h.set(1, HintState::Waiting, w(WaitKind::IdleInput, "Claude is waiting for your input"));
        assert!(h.waiting(1, u64::MAX, 60_000).is_none());
        // the same permission asked again after being answered is a new wait the parent hears of again
        wait(&h);
        assert!(h.take_untold(1).is_some());
        h.set(1, HintState::Working, None);
        wait(&h);
        assert!(h.take_untold(1).is_some());
    }

    #[test]
    fn kinds_round_trip_and_text_is_cleaned() {
        for k in [WaitKind::Permission, WaitKind::Question, WaitKind::IdleInput] {
            assert_eq!(WaitKind::parse(k.as_str()), Some(k));
        }
        assert_eq!(WaitKind::parse("screen"), None);
        assert_eq!(clean_text("  a\x1b[31m b\r\n"), "a[31m b");
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

    #[test]
    fn await_holds_until_input_output_or_expiry() {
        let lim = await_limit_ms(None);
        assert_eq!(lim, AWAIT_CAP_MS);
        assert_eq!(await_limit_ms(Some(10)), 600_000);
        assert_eq!(await_limit_ms(Some(0)), 60_000);
        assert_eq!(await_limit_ms(Some(100_000)), AWAIT_CAP_MS);
        // quiet since before the call
        assert!(await_active(10_000, lim, u64::MAX, 10_000));
        // the agent's closing words right after the call
        assert!(await_active(10_000, lim, u64::MAX, 2_000));
        // real output long after the call: it works again
        assert!(!await_active(100_000, lim, u64::MAX, 5_000));
        // the user typed after the call
        assert!(!await_active(10_000, lim, 3_000, 10_000));
        // expired by --for-min and by the cap
        assert!(!await_active(700_000, 600_000, u64::MAX, 700_000));
        assert!(!await_active(AWAIT_CAP_MS, lim, u64::MAX, AWAIT_CAP_MS));
    }

    #[test]
    fn awaits_set_show_and_clear() {
        let a = Awaits::default();
        assert!(a.active(1, u64::MAX, 0).is_none());
        a.set(1, "  CI #3703 \u{7}", Some(5));
        let v = a.active(1, u64::MAX, 0).unwrap();
        assert_eq!(v.reason, "CI #3703");
        assert_eq!(v.until - v.since, 300_000);
        assert!(a.active(2, u64::MAX, 0).is_none());
        assert!(a.clear(1));
        assert!(!a.clear(1));
        assert!(a.active(1, u64::MAX, 0).is_none());
    }
}
