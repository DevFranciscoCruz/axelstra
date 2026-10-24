//! `axelstra hook claude|gemini|codex EVENT`: called by Claude Code's hooks (Notification, Stop, UserPromptSubmit),
//! Gemini CLI's (Notification, BeforeAgent, AfterAgent, AfterTool) or Codex's (PermissionRequest, PreToolUse, Stop,
//! UserPromptSubmit) inside a session Axelstra started. It tells the daemon whether the agent waits for the user (with
//! the kind and the agent's own text), is idle or is working. That is the only source of the "waiting" state besides
//! `axelstra ask`/`escalate`: nothing is read off the screen. It never fails and gives up fast; it prints nothing,
//! except `{}` for Gemini, whose hooks must answer with JSON.

use axelstra_daemon::{client::Client, proto};
use serde_json::{json, Value};
use std::time::Duration;

/// Agents whose hook calls `axelstra hook <agent> EVENT` understands. Codex has no installer here: its hooks live in
/// `~/.codex/hooks.json` and each one must be trusted by the user inside Codex.
const HOOK_AGENTS: [&str; 3] = ["claude", "gemini", "codex"];

/// What a hook event tells the daemon: the state, and for a wait its kind and the agent's own text.
#[derive(Debug, PartialEq)]
pub struct Event {
    pub state: &'static str,
    pub kind: Option<&'static str>,
    pub text: String,
}

fn plain(state: &'static str) -> Option<Event> {
    Some(Event { state, kind: None, text: String::new() })
}

fn wait(kind: &'static str, text: String) -> Option<Event> {
    Some(Event { state: "waiting", kind: Some(kind), text })
}

/// The first non-empty string among `keys` of `v`.
fn field<'a>(v: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter().find_map(|k| v[k].as_str().map(str::trim).filter(|s| !s.is_empty()))
}

/// The event as the daemon should hear it, from the agent, the hook event name and the JSON the agent put on stdin
/// (`Null` when there was none). A wait needs a kind the agent itself named: a notification of a type we do not know
/// (Claude's `auth_success`, ...) says nothing, and nothing is ever guessed from the text.
/// Verified: Claude Code 2.1.310 has the Notification types `permission_prompt`, `idle_prompt` and
/// `elicitation_dialog`; Gemini CLI 0.65.0 sends `ToolPermission` (fixture in tests/fixtures).
pub fn event_of(agent: &str, event: &str, payload: &Value) -> Option<Event> {
    let message = field(payload, &["message"]).unwrap_or_default();
    match (agent, event.to_ascii_lowercase().as_str()) {
        ("claude", "notification") => match field(payload, &["notification_type"])? {
            "permission_prompt" => wait("permission", message.into()),
            "elicitation_dialog" => wait("question", message.into()),
            // it finished and sits at the prompt: the Stop hook already said idle, this is not a wait
            "idle_prompt" => wait("idle_input", message.into()),
            _ => None,
        },
        ("gemini", "notification") => match field(payload, &["notification_type"])? {
            "ToolPermission" => {
                let cmd = field(&payload["details"], &["command", "title"]).unwrap_or_default();
                wait("permission", if cmd.is_empty() { message.into() } else { format!("{message}: {cmd}") })
            }
            _ => None,
        },
        // Codex: PermissionRequest is the approval prompt; a tool running (PreToolUse) means it was answered
        ("codex", "permissionrequest") => {
            let tool = field(payload, &["tool_name"]).unwrap_or("tool");
            let what = field(&payload["tool_input"], &["command", "description"]).unwrap_or_default();
            wait("permission", if what.is_empty() { tool.into() } else { format!("{tool}: {what}") })
        }
        ("codex", "pretooluse") => plain("working"),
        ("claude" | "codex", "stop") => plain("idle"),
        ("claude" | "codex", "userpromptsubmit") => plain("working"),
        // Gemini CLI: a prompt was submitted / the turn ended / a tool ran, so its permission prompt was answered
        ("gemini", "beforeagent" | "aftertool") => plain("working"),
        ("gemini", "afteragent") => plain("idle"),
        _ => None,
    }
}

/// Events whose stdin JSON carries the text of a wait.
fn reads_payload(event: &str) -> bool {
    matches!(event.to_ascii_lowercase().as_str(), "notification" | "permissionrequest")
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
    let events = axelstra_daemon::hooks::events_of(agent);
    let missing: Vec<&str> = events.iter().copied().filter(|e| v["events"][e] != json!(true)).collect();
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
    let Ok(session) = std::env::var("AXELSTRA_SESSION") else { return };
    let Ok(id) = session.trim().parse::<u32>() else { return };
    if !HOOK_AGENTS.contains(&agent) {
        return;
    }
    let (agent, event) = (agent.to_string(), event.to_string());
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let sent = (|| {
            let payload = if reads_payload(&event) { read_payload() } else { Value::Null };
            let ev = event_of(&agent, &event, &payload)?;
            let mut c = Client::connect_cli(&proto::pipe_name(), &proto::load_or_create_token().ok()?).ok()?;
            c.call("session.hint", json!({"id": id, "state": ev.state, "kind": ev.kind, "text": ev.text})).ok()
        })();
        let _ = tx.send(sent.is_some());
    });
    let _ = rx.recv_timeout(LIMIT);
}

/// The JSON the agent writes to the hook's stdin (at most 1 MiB); `Null` when there is none or it does not parse.
fn read_payload() -> Value {
    use std::io::Read;
    let mut buf = String::new();
    let _ = std::io::stdin().take(1 << 20).read_to_string(&mut buf);
    serde_json::from_str(&buf).unwrap_or(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(agent: &str, event: &str, payload: Value) -> Option<(&'static str, Option<&'static str>, String)> {
        event_of(agent, event, &payload).map(|e| (e.state, e.kind, e.text))
    }

    #[test]
    fn claude_notifications_are_waits_only_by_their_own_type() {
        let n = |t: &str| json!({"notification_type": t, "message": "Claude needs your permission to use Bash"});
        assert_eq!(
            ev("claude", "Notification", n("permission_prompt")),
            Some(("waiting", Some("permission"), "Claude needs your permission to use Bash".into()))
        );
        assert_eq!(ev("claude", "Notification", n("elicitation_dialog")).unwrap().1, Some("question"));
        assert_eq!(ev("claude", "Notification", n("idle_prompt")).unwrap().1, Some("idle_input"));
        // unknown type, or no type at all (old Claude, a payload that did not arrive): says nothing
        assert_eq!(ev("claude", "Notification", n("auth_success")), None);
        assert_eq!(ev("claude", "Notification", json!({"message": "Do you want to proceed?"})), None);
        assert_eq!(ev("claude", "Notification", Value::Null), None);
        assert_eq!(ev("claude", "Stop", Value::Null), Some(("idle", None, String::new())));
        assert_eq!(ev("claude", "UserPromptSubmit", Value::Null).unwrap().0, "working");
        assert_eq!(ev("claude", "PreToolUse", Value::Null), None);
    }

    #[test]
    fn gemini_permission_notification_carries_the_command() {
        let p = json!({"notification_type": "ToolPermission", "message": "Tool Confirm Shell Command requires execution", "details": {"command": "mkdir newdir_stub"}});
        let e = ev("gemini", "Notification", p).unwrap();
        assert_eq!((e.0, e.1), ("waiting", Some("permission")));
        assert!(e.2.ends_with(": mkdir newdir_stub"), "{}", e.2);
        assert_eq!(ev("gemini", "Notification", json!({"notification_type": "Other"})), None);
        assert_eq!((ev("gemini", "BeforeAgent", Value::Null).unwrap().0, ev("gemini", "AfterAgent", Value::Null).unwrap().0), ("working", "idle"));
        assert_eq!(ev("gemini", "AfterTool", Value::Null).unwrap().0, "working");
        assert_eq!(ev("gemini", "BeforeTool", Value::Null), None);
    }

    #[test]
    fn codex_approvals_wait_and_a_tool_run_means_answered() {
        let p = json!({"tool_name": "shell", "tool_input": {"command": "rm -rf build"}});
        assert_eq!(ev("codex", "PermissionRequest", p), Some(("waiting", Some("permission"), "shell: rm -rf build".into())));
        assert_eq!(ev("codex", "PreToolUse", Value::Null).unwrap().0, "working");
        assert_eq!(ev("codex", "Stop", Value::Null).unwrap().0, "idle");
        // another agent's event name means nothing for this one
        assert_eq!(ev("claude", "PermissionRequest", Value::Null), None);
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
        let st = json!({"path": "p", "events": {"Notification": true, "AfterAgent": false, "BeforeAgent": true, "AfterTool": true}});
        let r = report("status", "gemini", &st);
        assert!(r.contains("Faltam hooks em p: AfterAgent") && r.ends_with("axelstra hook install gemini"), "{r}");
    }

    #[test]
    fn without_a_session_it_does_nothing_and_returns() {
        std::env::remove_var("AXELSTRA_SESSION");
        run("claude", "Stop");
        run("gemini", "AfterAgent");
        run("codex", "PermissionRequest");
    }
}
