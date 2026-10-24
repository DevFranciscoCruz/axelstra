//! Installs the Axelstra status hooks into Claude Code's `~/.claude/settings.json` (Notification, Stop,
//! UserPromptSubmit) or Gemini CLI's `~/.gemini/settings.json` (Notification, BeforeAgent, AfterAgent, AfterTool), so the user
//! does not paste a snippet by hand. Both files share the `hooks` shape; only the events and the agent name differ. The file belongs to the user and to other
//! tools: it is merged, never rewritten from a template. Nothing is removed or reordered (serde_json keeps key
//! order, see `preserve_order`), a timestamped backup is written first, the write is atomic, and a file that is not
//! valid JSON is refused untouched.

use crate::store;
use anyhow::{bail, Context, Result};
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};

/// Claude Code hook events Axelstra listens to, as `axelstra hook claude EVENT` understands them.
pub const EVENTS: [&str; 3] = ["Notification", "Stop", "UserPromptSubmit"];
/// Gemini CLI hook events, all verified firing in gemini-cli 0.65.0 (a real install against a stub model server):
/// a prompt was submitted (BeforeAgent), the turn ended (AfterAgent), a tool permission is asked (Notification, type
/// ToolPermission), a tool finished (AfterTool: the permission was answered, back to working).
pub const GEMINI_EVENTS: [&str; 4] = ["Notification", "AfterAgent", "BeforeAgent", "AfterTool"];

/// The agents whose settings Axelstra can write hooks into.
pub const AGENTS: [&str; 2] = ["claude", "gemini"];

pub fn events_of(agent: &str) -> &'static [&'static str] {
    if agent == "gemini" {
        &GEMINI_EVENTS
    } else {
        &EVENTS
    }
}

fn check_agent(agent: &str) -> Result<()> {
    if !AGENTS.contains(&agent) {
        bail!("só o claude e o gemini têm hooks");
    }
    Ok(())
}

fn home() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from)
}

/// Claude Code's user-level settings file.
pub fn settings_path() -> Result<PathBuf> {
    settings_path_of("claude")
}

/// The user-level settings file of `agent` (`~/.claude/settings.json`, `~/.gemini/settings.json`).
pub fn settings_path_of(agent: &str) -> Result<PathBuf> {
    check_agent(agent)?;
    Ok(home().context("no home directory")?.join(format!(".{agent}")).join("settings.json"))
}

/// The command Claude runs for `event`: this executable by absolute path, so the hook never depends on what sits first
/// on PATH (it receives the agent's prompts). Falls back to the bare name only when the path is unknown.
fn command(agent: &str, event: &str) -> String {
    // Gemini runs a hook through PowerShell on Windows (bash elsewhere): there a quoted path alone is a string, not a call
    command_for(agent, event, cfg!(windows) && agent == "gemini")
}

fn command_for(agent: &str, event: &str, powershell: bool) -> String {
    match std::env::current_exe() {
        Ok(exe) if powershell => format!("& '{}' hook {agent} {event}", exe.display().to_string().replace('\'', "''")),
        Ok(exe) => format!("\"{}\" hook {agent} {event}", exe.display()),
        Err(_) => format!("axelstra hook {agent} {event}"),
    }
}

/// True for a hook command that is ours for `event`, whether written by an earlier release (bare `axelstra`) or by this one.
fn is_ours(cmd: &str, agent: &str, event: &str) -> bool {
    let Some(prog) = cmd.trim().strip_suffix(&format!(" hook {agent} {event}")) else { return false };
    let prog = prog.trim().trim_start_matches('&').trim().trim_matches(['"', '\'']);
    let name = prog.rsplit(['/', '\\']).next().unwrap_or(prog).to_ascii_lowercase();
    name.starts_with("axelstra")
}

/// The file as JSON: a missing or blank file is an empty object, anything that is not an object is refused.
/// The bytes are returned too (None: the file does not exist) so the backup is exact.
fn load(path: &Path) -> Result<(Value, Option<Vec<u8>>)> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok((json!({}), None)),
        Err(e) => return Err(e).with_context(|| format!("ler {}", path.display())),
    };
    let text = String::from_utf8_lossy(&bytes);
    let text = text.trim_start_matches('\u{feff}');
    if text.trim().is_empty() {
        return Ok((json!({}), Some(bytes)));
    }
    let v: Value = serde_json::from_str(text)
        .map_err(|e| anyhow::anyhow!("{} não é JSON válido ({e}); não lhe toquei. Corrige-o à mão e tenta outra vez.", path.display()))?;
    if !v.is_object() {
        bail!("{} não é um objeto JSON; não lhe toquei.", path.display());
    }
    Ok((v, Some(bytes)))
}

/// True when some entry of `list` already runs the Axelstra hook command for `event`.
fn entry_present(list: &[Value], agent: &str, event: &str) -> bool {
    list.iter()
        .any(|e| e["hooks"].as_array().is_some_and(|hs| hs.iter().any(|h| h["command"].as_str().is_some_and(|c| is_ours(c, agent, event)))))
}

fn has_event(root: &Value, agent: &str, event: &str) -> bool {
    root["hooks"][event].as_array().is_some_and(|list| entry_present(list, agent, event))
}

fn status_of(agent: &str, path: &Path, root: &Value, exists: bool) -> Value {
    let events: Map<String, Value> = events_of(agent).iter().map(|e| (e.to_string(), json!(has_event(root, agent, e)))).collect();
    json!({
        "path": path.to_string_lossy(),
        "exists": exists,
        "installed": events.values().all(|v| v == &json!(true)),
        "events": events,
    })
}

/// Are the three hooks present in the settings file at `path`. A file that cannot be read as JSON is an answer the
/// panel can show (`problem`), not a crash.
pub fn status_at(path: &Path) -> Value {
    status_at_of("claude", path)
}

pub fn status_at_of(agent: &str, path: &Path) -> Value {
    match load(path) {
        Ok((root, bytes)) => status_of(agent, path, &root, bytes.is_some()),
        Err(e) => {
            let mut v = status_of(agent, path, &json!({}), true);
            v["problem"] = json!(e.to_string());
            v
        }
    }
}

pub fn status() -> Result<Value> {
    status_for("claude")
}

pub fn status_for(agent: &str) -> Result<Value> {
    Ok(status_at_of(agent, &settings_path_of(agent)?))
}

/// Adds the missing hook entries to `root`; true when something was added. Fails, touching nothing on disk, when
/// `hooks` or one of the events has an unexpected shape.
fn merge(agent: &str, root: &mut Value, path: &Path) -> Result<bool> {
    let obj = root.as_object_mut().context("settings root is not an object")?;
    let hooks = obj.entry("hooks").or_insert_with(|| json!({}));
    let Some(hooks) = hooks.as_object_mut() else { bail!("\"hooks\" em {} não é um objeto; não lhe toquei.", path.display()) };
    let mut changed = false;
    for event in events_of(agent).iter().copied() {
        let list = hooks.entry(event).or_insert_with(|| json!([]));
        let Some(list) = list.as_array_mut() else { bail!("\"hooks.{event}\" em {} não é uma lista; não lhe toquei.", path.display()) };
        // an entry written by an earlier release names `axelstra` bare: it is pointed at this executable
        let want = command(agent, event);
        for h in list.iter_mut().filter_map(|e| e["hooks"].as_array_mut()).flatten() {
            if h["command"].as_str().is_some_and(|c| is_ours(c, agent, event) && c != want) {
                h["command"] = json!(want);
                changed = true;
            }
        }
        if !entry_present(list, agent, event) {
            let mut hook = json!({ "type": "command", "command": command(agent, event) });
            if agent == "gemini" {
                // shown by Gemini's `/hooks panel`; the hook itself bounds its run time (1.5 s)
                hook["name"] = json!(format!("axelstra-{}", event.to_ascii_lowercase()));
            }
            list.push(json!({ "hooks": [hook] }));
            changed = true;
        }
    }
    Ok(changed)
}

/// Merges the hooks into the file at `path`. Returns the new status plus `changed` and the `backup` path (null when
/// there was nothing to back up or nothing to change). A second call changes nothing and writes nothing.
pub fn install_at(path: &Path) -> Result<Value> {
    install_at_of("claude", path)
}

pub fn install_at_of(agent: &str, path: &Path) -> Result<Value> {
    check_agent(agent)?;
    let (mut root, original) = load(path)?;
    if !merge(agent, &mut root, path)? {
        let mut v = status_of(agent, path, &root, original.is_some());
        v["changed"] = json!(false);
        v["backup"] = Value::Null;
        return Ok(v);
    }
    let mut backup = Value::Null;
    if let Some(bytes) = original.as_ref().filter(|b| !b.is_empty()) {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let b = path.with_file_name(format!("{name}.axelstra-backup-{}", chrono::Local::now().format("%Y%m%d-%H%M%S%3f")));
        store::write_atomic(&b, bytes).with_context(|| format!("escrever a cópia de segurança {}", b.display()))?;
        backup = json!(b.to_string_lossy());
    }
    let mut text = serde_json::to_string_pretty(&root)?;
    text.push('\n');
    store::write_atomic(path, text.as_bytes()).with_context(|| format!("escrever {}", path.display()))?;
    let mut v = status_of(agent, path, &root, true);
    v["changed"] = json!(true);
    v["backup"] = backup;
    Ok(v)
}

pub fn install() -> Result<Value> {
    install_for("claude")
}

pub fn install_for(agent: &str) -> Result<Value> {
    install_at_of(agent, &settings_path_of(agent)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("axl-hooks-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d.join("settings.json")
    }

    fn backups(f: &Path) -> Vec<PathBuf> {
        std::fs::read_dir(f.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.to_string_lossy().contains("axelstra-backup"))
            .collect()
    }

    fn read(f: &Path) -> Value {
        serde_json::from_str(&std::fs::read_to_string(f).unwrap()).unwrap()
    }

    #[test]
    fn missing_file_is_created_with_the_three_hooks() {
        let f = tmp("missing");
        assert_eq!(status_at(&f)["installed"], false);
        assert_eq!(status_at(&f)["exists"], false);
        let r = install_at(&f).unwrap();
        assert_eq!((r["changed"].clone(), r["installed"].clone(), r["backup"].clone()), (json!(true), json!(true), Value::Null));
        assert_eq!(read(&f)["hooks"]["Stop"][0]["hooks"][0]["command"], command("claude", "Stop"));
        assert_eq!(status_at(&f)["installed"], true);
        let _ = std::fs::remove_dir_all(f.parent().unwrap());
    }

    #[test]
    fn empty_file_is_filled_without_a_backup_of_nothing() {
        let f = tmp("empty");
        std::fs::write(&f, "  \n").unwrap();
        let r = install_at(&f).unwrap();
        assert_eq!(r["installed"], true);
        assert_eq!(read(&f)["hooks"].as_object().unwrap().len(), 3);
        let _ = std::fs::remove_dir_all(f.parent().unwrap());
    }

    #[test]
    fn other_settings_and_hooks_are_kept_in_place() {
        let f = tmp("others");
        let before = r#"{
  "theme": "dark",
  "hooks": {
    "Stop": [{ "matcher": "x", "hooks": [{ "type": "command", "command": "notify-send done" }] }],
    "PreToolUse": [{ "hooks": [{ "type": "command", "command": "lint" }] }]
  },
  "model": "opus"
}"#;
        std::fs::write(&f, before).unwrap();
        let r = install_at(&f).unwrap();
        assert_eq!(r["changed"], true);
        let b = backups(&f);
        assert_eq!(b.len(), 1);
        assert_eq!(std::fs::read_to_string(&b[0]).unwrap(), before);
        let v = read(&f);
        let keys: Vec<_> = v.as_object().unwrap().keys().cloned().collect();
        assert_eq!(keys, ["theme", "hooks", "model"]);
        let hook_keys: Vec<_> = v["hooks"].as_object().unwrap().keys().cloned().collect();
        assert_eq!(hook_keys, ["Stop", "PreToolUse", "Notification", "UserPromptSubmit"]);
        // the other tool's Stop hook stays first, ours follows
        assert_eq!(v["hooks"]["Stop"][0]["hooks"][0]["command"], "notify-send done");
        assert_eq!(v["hooks"]["Stop"][1]["hooks"][0]["command"], command("claude", "Stop"));
        assert_eq!(v["hooks"]["PreToolUse"][0]["hooks"][0]["command"], "lint");
        let _ = std::fs::remove_dir_all(f.parent().unwrap());
    }

    #[test]
    fn second_install_is_a_no_op() {
        let f = tmp("twice");
        std::fs::write(&f, r#"{"a":1}"#).unwrap();
        install_at(&f).unwrap();
        let once = std::fs::read_to_string(&f).unwrap();
        let r = install_at(&f).unwrap();
        assert_eq!((r["changed"].clone(), r["backup"].clone()), (json!(false), Value::Null));
        assert_eq!(std::fs::read_to_string(&f).unwrap(), once);
        assert_eq!(backups(&f).len(), 1);
        let _ = std::fs::remove_dir_all(f.parent().unwrap());
    }

    #[test]
    fn a_bare_axelstra_hook_is_pointed_at_this_executable() {
        let f = tmp("legacy");
        std::fs::write(&f, r#"{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"axelstra hook claude Stop"}]}]}}"#).unwrap();
        let r = install_at(&f).unwrap();
        assert_eq!(r["changed"], true);
        let v = read(&f);
        assert_eq!(v["hooks"]["Stop"].as_array().unwrap().len(), 1);
        assert_eq!(v["hooks"]["Stop"][0]["hooks"][0]["command"], command("claude", "Stop"));
        assert!(command("claude", "Stop").starts_with('"'));
        assert!(is_ours("axelstra hook claude Stop", "claude", "Stop"));
        assert!(is_ours(&command("claude", "Stop"), "claude", "Stop"));
        assert!(!is_ours("evil hook claude Stop", "claude", "Stop"));
        let _ = std::fs::remove_dir_all(f.parent().unwrap());
    }

    #[test]
    fn invalid_json_is_refused_and_left_alone() {
        let f = tmp("invalid");
        std::fs::write(&f, "{ not json").unwrap();
        let e = install_at(&f).unwrap_err().to_string();
        assert!(e.contains("não é JSON válido"), "{e}");
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "{ not json");
        assert!(backups(&f).is_empty());
        assert!(status_at(&f)["problem"].is_string());
        // valid JSON of the wrong shape is refused too
        std::fs::write(&f, "[1]").unwrap();
        assert!(install_at(&f).is_err());
        std::fs::write(&f, r#"{"hooks":{"Stop":"x"}}"#).unwrap();
        assert!(install_at(&f).is_err());
        assert_eq!(std::fs::read_to_string(&f).unwrap(), r#"{"hooks":{"Stop":"x"}}"#);
        assert!(backups(&f).is_empty());
        let _ = std::fs::remove_dir_all(f.parent().unwrap());
    }

    #[test]
    fn status_reports_each_event() {
        let f = tmp("partial");
        std::fs::write(&f, r#"{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"axelstra hook claude Stop"}]}]}}"#).unwrap();
        let s = status_at(&f);
        assert_eq!(
            (s["installed"].clone(), s["events"]["Stop"].clone(), s["events"]["Notification"].clone()),
            (json!(false), json!(true), json!(false))
        );
        let _ = std::fs::remove_dir_all(f.parent().unwrap());
    }

    #[test]
    fn a_status_line_in_the_settings_is_neither_replaced_nor_duplicated() {
        let f = tmp("statusline");
        let before = r#"{"statusLine":{"type":"command","command":"ccstatusline","padding":0},"model":"opus"}"#;
        std::fs::write(&f, before).unwrap();
        install_at(&f).unwrap();
        install_at(&f).unwrap();
        let v = read(&f);
        assert_eq!(v["statusLine"], json!({"type":"command","command":"ccstatusline","padding":0}));
        assert_eq!(std::fs::read_to_string(&f).unwrap().matches("statusLine").count(), 1);
        let _ = std::fs::remove_dir_all(f.parent().unwrap());
    }

    #[test]
    fn gemini_hooks_go_to_their_own_events_and_keep_the_users_settings() {
        let f = tmp("gemini");
        let before = r#"{"general":{"vimMode":true},"hooks":{"AfterAgent":[{"hooks":[{"type":"command","command":"my-notify"}]}]}}"#;
        std::fs::write(&f, before).unwrap();
        assert_eq!(status_at_of("gemini", &f)["installed"], false);
        let r = install_at_of("gemini", &f).unwrap();
        assert_eq!((r["changed"].clone(), r["installed"].clone()), (json!(true), json!(true)));
        let v = read(&f);
        let keys: Vec<_> = v["hooks"].as_object().unwrap().keys().cloned().collect();
        assert_eq!(keys, ["AfterAgent", "Notification", "BeforeAgent", "AfterTool"]);
        assert_eq!(v["hooks"]["AfterTool"][0]["hooks"][0]["name"], "axelstra-aftertool");
        assert_eq!(v["hooks"]["AfterAgent"][0]["hooks"][0]["command"], "my-notify");
        assert_eq!(v["hooks"]["AfterAgent"][1]["hooks"][0]["command"], command("gemini", "AfterAgent"));
        assert!(v["hooks"].get("Stop").is_none() && v["general"]["vimMode"] == true);
        assert_eq!(backups(&f).len(), 1);
        // second run is a no-op, and a claude hook command is not mistaken for a gemini one
        assert_eq!(install_at_of("gemini", &f).unwrap()["changed"], false);
        assert!(!is_ours(&command("claude", "Notification"), "gemini", "Notification"));
        let _ = std::fs::remove_dir_all(f.parent().unwrap());
    }

    #[test]
    fn gemini_hooks_on_windows_are_a_powershell_call_not_a_bare_string() {
        // Gemini 0.65 runs a hook as `powershell -NoProfile -Command <command>` on Windows, where a quoted path
        // followed by arguments is a parse error: the command is a call (`&`) of a single-quoted path
        let ps = command_for("gemini", "AfterAgent", true);
        assert!(ps.starts_with("& '") && ps.ends_with("' hook gemini AfterAgent"), "{ps}");
        let sh = command_for("gemini", "AfterAgent", false);
        assert!(sh.starts_with('"') && sh.ends_with("\" hook gemini AfterAgent"), "{sh}");
        // either spelling is ours, whatever was installed before, and a lookalike is not
        assert!(is_ours(&ps, "gemini", "AfterAgent") && is_ours(&sh, "gemini", "AfterAgent"));
        assert!(is_ours(r"& 'C:\Program Files\Axelstra\axelstra.exe' hook gemini AfterTool", "gemini", "AfterTool"));
        assert!(!is_ours(r"& 'C:\evil\rm.exe' hook gemini AfterTool", "gemini", "AfterTool"));
    }

    #[test]
    fn an_older_quoted_gemini_command_is_rewritten_not_duplicated() {
        let f = tmp("gemini-old");
        let old =
            r#"{"hooks":{"AfterAgent":[{"hooks":[{"type":"command","command":"\"C:\\old\\axelstra.exe\" hook gemini AfterAgent"}]}]}}"#;
        std::fs::write(&f, old).unwrap();
        let r = install_at_of("gemini", &f).unwrap();
        assert_eq!(r["changed"], true);
        let v = read(&f);
        assert_eq!(v["hooks"]["AfterAgent"].as_array().unwrap().len(), 1);
        assert_eq!(v["hooks"]["AfterAgent"][0]["hooks"][0]["command"], command("gemini", "AfterAgent"));
        let _ = std::fs::remove_dir_all(f.parent().unwrap());
    }

    #[test]
    fn only_claude_and_gemini_have_a_settings_file() {
        assert!(settings_path_of("codex").is_err());
        assert!(install_at_of("codex", Path::new("x")).is_err());
        assert!(settings_path_of("gemini").unwrap().ends_with(".gemini/settings.json") || cfg!(windows));
    }
}
