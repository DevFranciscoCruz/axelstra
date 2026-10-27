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
}

/// True when the agent has been quiet (no output, no typing) for `threshold_ms` with nothing that explains it (0 = off).
pub fn is_stalled(f: &Facts, threshold_ms: u64) -> bool {
    threshold_ms > 0
        && f.agent
        && !f.main
        && f.running
        && !f.reported
        && !f.escalated
        && !f.awaiting
        && !f.live_children
        && f.idle_ms.min(f.since_input_ms) >= threshold_ms
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
    let quiet = f.idle_ms.min(f.since_input_ms);
    (quiet < threshold_ms).then(|| threshold_ms - quiet + 20)
}

/// The English text typed into the parent's session (agent-facing, so it stays English).
pub fn parent_text(label: &str, id: u32, minutes: u64, last_line: &str) -> String {
    format!(
        "\"{label}\" (session {id}) has been stalled for {minutes} min without calling `axelstra done` or `axelstra escalate`; \
         it may be waiting for a decision. Last screen line: {last_line}. Answer it with `axelstra msg {id} \"<text>\"` \
         or check its screen with `axelstra read {id}`."
    )
}

/// The last non-empty line of a screen, trimmed and clipped (what the notification shows).
pub fn last_line(lines: &[String]) -> String {
    let l = lines.iter().rev().map(|l| l.trim()).find(|l| !l.is_empty()).unwrap_or_default();
    l.chars().take(200).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn quiet() -> Facts {
        Facts { agent: true, running: true, idle_ms: 6 * 60_000, since_input_ms: u64::MAX, ..Facts::default() }
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
    }

    #[test]
    fn parent_text_names_the_session_and_the_line() {
        let t = parent_text("Tarefa 2", 7, 5, "waiting for you");
        assert!(t.contains("Tarefa 2") && t.contains("session 7") && t.contains("5 min") && t.contains("waiting for you"));
        assert!(t.contains("axelstra msg 7"));
    }
}
