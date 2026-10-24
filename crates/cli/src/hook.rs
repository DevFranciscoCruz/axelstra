//! `axelstra hook claude EVENT` / `axelstra hook gemini EVENT`: called by Claude Code's hooks (Notification, Stop,
//! UserPromptSubmit) or Gemini CLI's (Notification, BeforeAgent, AfterAgent) inside a session Axelstra started. It tells the daemon whether the agent waits for the user, is idle or is working, so the sidebar
//! dot is right even when the screen shows no recognisable prompt. It never fails and gives up fast; it prints nothing, except `{}` for Gemini, whose hooks must answer with JSON.

use axelstra_daemon::{client::Client, proto};
use serde_json::json;
use std::time::Duration;

/// The state a Claude Code hook event stands for.
pub fn state_of(event: &str) -> Option<&'static str> {
    match event.to_ascii_lowercase().as_str() {
        "notification" => Some("waiting"),
        "stop" => Some("idle"),
        "userpromptsubmit" => Some("working"),
        // Gemini CLI: a prompt was submitted / the turn ended
        "beforeagent" => Some("working"),
        "afteragent" => Some("idle"),
        _ => None,
    }
}

/// Longest the hook may take, daemon down or hung included: a hook must never hold the agent up.
const LIMIT: Duration = Duration::from_millis(1500);

/// `axelstra hook install claude` and `axelstra hook status claude` manage the settings entries (they print and may
/// fail); everything else is the hook call itself, which never prints and never fails.
pub fn dispatch(agent: &str, event: &str) -> anyhow::Result<()> {
    match agent {
        "install" | "status" => {
            if !axelstra_daemon::hooks::AGENTS.contains(&event) {
                anyhow::bail!("só o claude e o gemini têm hooks: axelstra hook {agent} claude|gemini");
            }
            let v =
                if agent == "install" { axelstra_daemon::hooks::install_for(event)? } else { axelstra_daemon::hooks::status_for(event)? };
            println!("{}", report(agent, event, &v));
        }
        _ => run(agent, event),
    }
    Ok(())
}

/// The one-line pt-PT answer printed by `hook install` and `hook status`.
fn report(action: &str, agent: &str, v: &serde_json::Value) -> String {
    let path = v["path"].as_str().unwrap_or_default();
    if let Some(p) = v["problem"].as_str() {
        return p.to_string();
    }
    if action == "install" {
        return match (v["changed"] == json!(true), v["backup"].as_str()) {
            (false, _) => format!("Os hooks já estavam instalados em {path}."),
            (true, Some(b)) => format!("Hooks instalados em {path}. Cópia de segurança: {b}"),
            (true, None) => format!("Hooks instalados em {path}."),
        };
    }
    let events = if agent == "gemini" { axelstra_daemon::hooks::GEMINI_EVENTS } else { axelstra_daemon::hooks::EVENTS };
    let missing: Vec<&str> = events.into_iter().filter(|e| v["events"][e] != json!(true)).collect();
    if missing.is_empty() {
        format!("Hooks instalados em {path}.")
    } else {
        format!("Faltam hooks em {path}: {}. Corre: axelstra hook install {agent}", missing.join(", "))
    }
}

pub fn run(agent: &str, event: &str) {
    if agent == "gemini" {
        // Gemini parses a hook's stdout as JSON: an empty object means "no decision"
        println!("{{}}");
    }
    let (Some(state), Ok(session)) = (state_of(event), std::env::var("AXELSTRA_SESSION")) else { return };
    let Ok(id) = session.trim().parse::<u32>() else { return };
    if !axelstra_daemon::hooks::AGENTS.contains(&agent) {
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
        assert_eq!((state_of("BeforeAgent"), state_of("AfterAgent")), (Some("working"), Some("idle")));
    }

    #[test]
    fn reports_are_pt_pt_and_name_what_is_missing() {
        let st = json!({"path": "p", "events": {"Notification": true, "Stop": false, "UserPromptSubmit": true}});
        assert!(report("status", "claude", &st).contains("Faltam hooks em p: Stop"));
        let done = json!({"path": "p", "events": {"Notification": true, "Stop": true, "UserPromptSubmit": true}});
        assert_eq!(report("status", "claude", &done), "Hooks instalados em p.");
        assert!(report("install", "claude", &json!({"path": "p", "changed": false})).contains("já estavam"));
        assert!(report("install", "claude", &json!({"path": "p", "changed": true, "backup": "b"})).contains("Cópia de segurança: b"));
    }

    #[test]
    fn only_claude_and_gemini_have_managed_hooks() {
        assert!(dispatch("install", "codex").is_err());
        assert!(dispatch("status", "chatgpt").is_err());
    }

    #[test]
    fn a_gemini_report_names_its_own_missing_events() {
        let st = json!({"path": "p", "events": {"Notification": true, "AfterAgent": false, "BeforeAgent": true}});
        let r = report("status", "gemini", &st);
        assert!(r.contains("Faltam hooks em p: AfterAgent") && r.ends_with("axelstra hook install gemini"), "{r}");
    }

    #[test]
    fn without_a_session_it_does_nothing_and_returns() {
        std::env::remove_var("AXELSTRA_SESSION");
        run("claude", "Stop");
        run("gemini", "AfterAgent");
    }
}
