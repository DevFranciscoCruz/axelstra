//! The terminal history an agent session leaves on disk for the next start to show dimmed.

use super::*;

pub(super) fn history_file(reg: &Registry, key: &str) -> Option<std::path::PathBuf> {
    // the key is a uuid; anything else would be a path
    key.chars().all(|c| c.is_ascii_alphanumeric() || c == '-').then(|| reg.saved.with_file_name("scrollback").join(format!("{key}.txt")))
}

pub(super) fn save_history(reg: &Registry, key: &str, lines: &[String]) {
    let Some(f) = history_file(reg, key) else { return };
    if lines.is_empty() {
        return;
    }
    let _ = write_atomic(&f, lines.join("\n").as_bytes());
}

/// The saved history as terminal text (dim, with a header), removed from disk once read.
pub(super) fn take_history(reg: &Registry, key: &str) -> Option<String> {
    let f = history_file(reg, key)?;
    let text = std::fs::read_to_string(&f).ok()?;
    let _ = std::fs::remove_file(&f);
    let body = text.lines().collect::<Vec<_>>().join("\r\n");
    Some(format!("\x1b[2m── histórico anterior ──\r\n{body}\r\n── fim do histórico ──\x1b[0m\r\n"))
}

/// Saves the screens of these sessions (agents with a history key only), for the next start to show dimmed.
pub(super) fn save_histories_of(reg: &Registry, ids: &[u32]) {
    if ids.is_empty() {
        return;
    }
    let picked: Vec<(String, Arc<Session>)> = {
        let sessions = reg.sessions.lock_recover();
        let metas = reg.metas.lock_recover();
        ids.iter().filter_map(|id| Some((metas.get(id)?["key"].as_str()?.to_string(), sessions.get(id)?.clone()))).collect()
    };
    for (key, s) in picked {
        save_history(reg, &key, &s.screen_lines(300));
    }
}

/// Writes the screen of every session that has a history key (agents), for the next start to show dimmed.
/// The locks (sessions, then metas) are held only to pick the sessions; rendering the screens happens after.
pub(super) fn save_histories(reg: &Registry) {
    let picked: Vec<(String, Arc<Session>)> = {
        let sessions = reg.sessions.lock_recover();
        let metas = reg.metas.lock_recover();
        sessions.iter().filter_map(|(id, s)| metas.get(id).and_then(|m| m["key"].as_str()).map(|k| (k.to_string(), s.clone()))).collect()
    };
    for (key, s) in picked {
        save_history(reg, &key, &s.screen_lines(300));
    }
}
