//! Installs the three Axelstra status hooks (Notification, Stop, UserPromptSubmit) into Claude Code's
//! `~/.claude/settings.json`, so the user does not paste a snippet by hand. The file belongs to the user and to other
//! tools: it is merged, never rewritten from a template. Nothing is removed or reordered (serde_json keeps key
//! order, see `preserve_order`), a timestamped backup is written first, the write is atomic, and a file that is not
//! valid JSON is refused untouched.

use crate::store;
use anyhow::{bail, Context, Result};
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};

/// Claude Code hook events Axelstra listens to, as `axelstra hook claude EVENT` understands them.
pub const EVENTS: [&str; 3] = ["Notification", "Stop", "UserPromptSubmit"];

fn home() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from)
}

/// Claude Code's user-level settings file.
pub fn settings_path() -> Result<PathBuf> {
    Ok(home().context("no home directory")?.join(".claude").join("settings.json"))
}

/// The command Claude runs for `event`: this executable by absolute path, so the hook never depends on what sits first
/// on PATH (it receives the agent's prompts). Falls back to the bare name only when the path is unknown.
fn command(event: &str) -> String {
    match std::env::current_exe() {
        Ok(exe) => format!("\"{}\" hook claude {event}", exe.display()),
        Err(_) => format!("axelstra hook claude {event}"),
    }
}

/// True for a hook command that is ours for `event`, whether written by an earlier release (bare `axelstra`) or by this one.
fn is_ours(cmd: &str, event: &str) -> bool {
    let Some(prog) = cmd.trim().strip_suffix(&format!(" hook claude {event}")) else { return false };
    let prog = prog.trim().trim_matches('"');
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
fn entry_present(list: &[Value], event: &str) -> bool {
    list.iter().any(|e| e["hooks"].as_array().is_some_and(|hs| hs.iter().any(|h| h["command"].as_str().is_some_and(|c| is_ours(c, event)))))
}

fn has_event(root: &Value, event: &str) -> bool {
    root["hooks"][event].as_array().is_some_and(|list| entry_present(list, event))
}

fn status_of(path: &Path, root: &Value, exists: bool) -> Value {
    let events: Map<String, Value> = EVENTS.iter().map(|e| (e.to_string(), json!(has_event(root, e)))).collect();
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
    match load(path) {
        Ok((root, bytes)) => status_of(path, &root, bytes.is_some()),
        Err(e) => {
            let mut v = status_of(path, &json!({}), true);
            v["problem"] = json!(e.to_string());
            v
        }
    }
}

pub fn status() -> Result<Value> {
    Ok(status_at(&settings_path()?))
}

/// Adds the missing hook entries to `root`; true when something was added. Fails, touching nothing on disk, when
/// `hooks` or one of the events has an unexpected shape.
fn merge(root: &mut Value, path: &Path) -> Result<bool> {
    let obj = root.as_object_mut().context("settings root is not an object")?;
    let hooks = obj.entry("hooks").or_insert_with(|| json!({}));
    let Some(hooks) = hooks.as_object_mut() else { bail!("\"hooks\" em {} não é um objeto; não lhe toquei.", path.display()) };
    let mut changed = false;
    for event in EVENTS {
        let list = hooks.entry(event).or_insert_with(|| json!([]));
        let Some(list) = list.as_array_mut() else { bail!("\"hooks.{event}\" em {} não é uma lista; não lhe toquei.", path.display()) };
        // an entry written by an earlier release names `axelstra` bare: it is pointed at this executable
        let want = command(event);
        for h in list.iter_mut().filter_map(|e| e["hooks"].as_array_mut()).flatten() {
            if h["command"].as_str().is_some_and(|c| is_ours(c, event) && c != want) {
                h["command"] = json!(want);
                changed = true;
            }
        }
        if !entry_present(list, event) {
            list.push(json!({ "hooks": [{ "type": "command", "command": command(event) }] }));
            changed = true;
        }
    }
    Ok(changed)
}

/// Merges the hooks into the file at `path`. Returns the new status plus `changed` and the `backup` path (null when
/// there was nothing to back up or nothing to change). A second call changes nothing and writes nothing.
pub fn install_at(path: &Path) -> Result<Value> {
    let (mut root, original) = load(path)?;
    if !merge(&mut root, path)? {
        let mut v = status_of(path, &root, original.is_some());
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
    let mut v = status_of(path, &root, true);
    v["changed"] = json!(true);
    v["backup"] = backup;
    Ok(v)
}

pub fn install() -> Result<Value> {
    install_at(&settings_path()?)
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
        assert_eq!(read(&f)["hooks"]["Stop"][0]["hooks"][0]["command"], command("Stop"));
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
        assert_eq!(v["hooks"]["Stop"][1]["hooks"][0]["command"], command("Stop"));
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
        assert_eq!(v["hooks"]["Stop"][0]["hooks"][0]["command"], command("Stop"));
        assert!(command("Stop").starts_with('"'));
        assert!(is_ours("axelstra hook claude Stop", "Stop"));
        assert!(is_ours(&command("Stop"), "Stop"));
        assert!(!is_ours("evil hook claude Stop", "Stop"));
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
}
