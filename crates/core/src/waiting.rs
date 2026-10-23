//! Detects an agent that stopped to ask the user something (a permission prompt, a y/n question).
//! Works on the tail of the terminal output; no parsing of the agent's own protocol.

/// Drops ANSI escape sequences (CSI, OSC) and carriage returns, keeping printable text.
pub fn strip_ansi(bytes: &[u8]) -> String {
    let s = String::from_utf8_lossy(bytes);
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '\x1b' => match it.peek() {
                Some('[') => {
                    it.next();
                    for n in it.by_ref() {
                        if ('@'..='~').contains(&n) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    it.next();
                    while let Some(n) = it.next() {
                        if n == '\x07' || (n == '\x1b' && it.peek() == Some(&'\\')) {
                            if n == '\x1b' {
                                it.next();
                            }
                            break;
                        }
                    }
                }
                _ => {
                    it.next();
                }
            },
            '\r' => {}
            c if c.is_control() && c != '\n' && c != '\t' => {}
            c => out.push(c),
        }
    }
    out
}

const MARKERS: &[&str] = &[
    "do you want to proceed",
    "do you want to make this edit",
    "do you want to create",
    "do you want to allow",
    "allow this",
    "(y/n)",
    "[y/n]",
    "[y/n/",
    "(yes/no)",
    "press enter to continue",
    "esc to cancel",
    "approve this",
    "waiting for your",
];

/// True when the last lines of the screen look like a question waiting for an answer.
pub fn needs_input(tail: &[u8]) -> bool {
    let text = strip_ansi(tail).to_lowercase();
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    let last: String = lines[lines.len().saturating_sub(12)..].join("\n");
    MARKERS.iter().any(|m| last.contains(m)) || (last.contains("❯ 1.") && last.contains("2."))
}

/// True when the screen's last lines (as rendered, see `Session::screen_lines`) contain a free-form question
/// the agent put to the user ("Uso X ou Y?"), which has no fixed marker. The TUI's own hints don't count.
pub fn asks_question(lines: &[String]) -> bool {
    let from = lines.len().saturating_sub(8);
    lines[from..].iter().any(|l| {
        let t = l.trim().trim_matches(|c: char| "│┃|>* ".contains(c)).trim();
        t.chars().count() >= 8 && t.ends_with('?') && !t.contains("for shortcuts") && !t.starts_with("? ")
    })
}

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

/// Latest hint per session, with the moment it arrived.
#[derive(Default)]
pub struct Hints(std::sync::Mutex<std::collections::HashMap<u32, (std::time::Instant, HintState)>>);

impl Hints {
    pub fn set(&self, id: u32, state: HintState) {
        if let Ok(mut m) = self.0.lock() {
            m.insert(id, (std::time::Instant::now(), state));
        }
    }

    pub fn clear(&self, id: u32) {
        if let Ok(mut m) = self.0.lock() {
            m.remove(&id);
        }
    }

    /// Whether the session's hint still says it waits, given how long it has been quiet (see `hint_waiting`).
    pub fn waiting(&self, id: u32, since_input_ms: u64, idle_ms: u64) -> bool {
        let Ok(m) = self.0.lock() else { return false };
        m.get(&id).map(|(at, st)| hint_waiting(*st, at.elapsed().as_millis() as u64, since_input_ms, idle_ms)).unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        h.set(1, HintState::Waiting);
        std::thread::sleep(std::time::Duration::from_millis(5));
        assert!(h.waiting(1, u64::MAX, 60_000));
        assert!(!h.waiting(2, u64::MAX, 60_000));
        h.set(1, HintState::Working);
        assert!(!h.waiting(1, u64::MAX, 60_000));
        h.set(1, HintState::Waiting);
        h.clear(1);
        assert!(!h.waiting(1, u64::MAX, 60_000));
        assert_eq!(HintState::parse("idle"), Some(HintState::Idle));
        assert_eq!(HintState::parse("x"), None);
    }

    #[test]
    fn free_form_question_is_detected() {
        let l = |s: &str| s.to_string();
        assert!(asks_question(&[l("Analisei o código."), l("● Uso a opção A ou a B?"), l("╭──╮"), l("│ > │")]));
        assert!(!asks_question(&[l("Tudo feito."), l("  ? for shortcuts")]));
        assert!(!asks_question(&[l("Terminei.")]));
    }

    #[test]
    fn claude_permission_prompt() {
        let screen =
            "\x1b[1mBash command\x1b[0m\n  npm install\n\n Do you want to proceed?\n \x1b[36m❯ 1. Yes\x1b[0m\n   2. No\n\n Esc to cancel";
        assert!(needs_input(screen.as_bytes()));
    }

    #[test]
    fn plain_question() {
        assert!(needs_input(b"Overwrite file? (y/n) "));
    }

    #[test]
    fn working_output_is_not_a_prompt() {
        assert!(!needs_input(b"\x1b[2KReading src/main.rs\nEditing 3 files\n"));
    }

    #[test]
    fn old_prompt_scrolled_away_is_ignored() {
        let mut s = String::from("Do you want to proceed?\n");
        for i in 0..30 {
            s.push_str(&format!("line {i}\n"));
        }
        assert!(!needs_input(s.as_bytes()));
    }
}
