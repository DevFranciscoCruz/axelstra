//! `axelstra hook claude EVENT`: called by Claude Code's hooks (Notification, Stop, UserPromptSubmit) inside a session
//! Axelstra started. It tells the daemon whether the agent waits for the user, is idle or is working, so the sidebar
//! dot is right even when the screen shows no recognisable prompt. It never prints, never fails and gives up fast.

use axelstra_daemon::{client::Client, proto};
use serde_json::json;
use std::time::Duration;

/// The state a Claude Code hook event stands for.
pub fn state_of(event: &str) -> Option<&'static str> {
    match event.to_ascii_lowercase().as_str() {
        "notification" => Some("waiting"),
        "stop" => Some("idle"),
        "userpromptsubmit" => Some("working"),
        _ => None,
    }
}

/// Longest the hook may take, daemon down or hung included: a hook must never hold the agent up.
const LIMIT: Duration = Duration::from_millis(1500);

pub fn run(agent: &str, event: &str) {
    let (Some(state), Ok(session)) = (state_of(event), std::env::var("AXELSTRA_SESSION")) else { return };
    let Ok(id) = session.trim().parse::<u32>() else { return };
    if agent != "claude" {
        return;
    }
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let sent = (|| {
            let mut c = Client::connect_cli(&proto::pipe_name(), &proto::load_or_create_token().ok()?).ok()?;
            c.call("session.hint", json!({"id": id, "state": state})).ok()
        })();
        let _ = tx.send(sent.is_some());
    });
    let _ = rx.recv_timeout(LIMIT);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_map_to_states() {
        assert_eq!(state_of("Notification"), Some("waiting"));
        assert_eq!(state_of("Stop"), Some("idle"));
        assert_eq!(state_of("UserPromptSubmit"), Some("working"));
        assert_eq!(state_of("PreToolUse"), None);
    }

    #[test]
    fn without_a_session_it_does_nothing_and_returns() {
        std::env::remove_var("AXELSTRA_SESSION");
        run("claude", "Stop");
    }
}
