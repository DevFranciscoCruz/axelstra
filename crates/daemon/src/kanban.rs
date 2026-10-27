//! Kanban cards: markdown files in `<repo>/.kanban/`. The folder is kept out of
//! git through `.git/info/exclude` (local, so the user's tracked files are not
//! touched) because cards are working notes, not project history.

use anyhow::Result;
use axelstra_core::agents;
use axelstra_core::card::{self, Card, Meta};
use axelstra_core::coded;
use serde::Serialize;
use std::path::{Path, PathBuf};

pub const STATUSES: [&str; 5] = ["backlog", "todo", "in-progress", "needs-review", "done"];

#[derive(Serialize)]
pub struct Board {
    pub cards: Vec<Card>,
    /// files that could not be parsed: shown so they are not silently lost
    pub broken: Vec<Broken>,
}

#[derive(Serialize)]
pub struct Broken {
    pub file: String,
    pub error: String,
    /// message code of `error` and its params, for the UI to translate (absent when the error has none)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_params: Option<std::collections::BTreeMap<String, String>>,
}

fn dir(repo: &Path) -> PathBuf {
    repo.join(".kanban")
}

pub fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

pub fn slug(title: &str) -> String {
    let mut s: String = title.to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    while s.contains("--") {
        s = s.replace("--", "-");
    }
    let s = s.trim_matches('-');
    let s: String = s.chars().take(40).collect();
    let s = s.trim_matches('-').to_string();
    if s.is_empty() {
        "card".into()
    } else {
        s
    }
}

pub fn list(repo: &Path) -> Result<Board> {
    let mut cards = Vec::new();
    let mut broken = Vec::new();
    if dir(repo).is_dir() {
        for e in std::fs::read_dir(dir(repo))? {
            let Ok(e) = e else { continue };
            let p = e.path();
            if p.extension().is_some_and(|x| x == "md") {
                let file = p.file_name().unwrap_or_default().to_string_lossy().to_string();
                // an unreadable file (permissions, not UTF-8) is reported, it does not hide the other cards
                let parsed = std::fs::read_to_string(&p).map_err(anyhow::Error::from).and_then(|t| card::parse(&t, &p));
                match parsed {
                    Ok(c) => cards.push(c),
                    Err(e) => {
                        let c = axelstra_core::msg::find(&e);
                        broken.push(Broken {
                            file,
                            error: format!("{e:#}"),
                            error_code: c.map(|c| c.code.clone()),
                            error_params: c.map(|c| c.params.clone()),
                        })
                    }
                }
            }
        }
    }
    cards.sort_by(|a, b| b.meta.priority.cmp(&a.meta.priority).then(a.meta.id.cmp(&b.meta.id)));
    Ok(Board { cards, broken })
}

pub(crate) fn exclude_kanban(repo: &Path) -> Result<()> {
    let common = axelstra_core::git::common_dir(repo)?;
    let f = common.join("info").join("exclude");
    let cur = std::fs::read_to_string(&f).unwrap_or_default();
    if cur.lines().any(|l| l.trim() == ".kanban/" || l.trim() == ".kanban") {
        return Ok(());
    }
    // already ignored by the repo's own .gitignore
    let ignored = axelstra_core::git::run(repo, &["check-ignore", "-q", ".kanban/x.md"]).map(|o| o.ok).unwrap_or(false);
    if ignored {
        return Ok(());
    }
    std::fs::create_dir_all(f.parent().unwrap())?;
    let mut next = cur;
    if !next.is_empty() && !next.ends_with('\n') {
        next.push('\n');
    }
    next.push_str(".kanban/\n");
    std::fs::write(&f, next)?;
    Ok(())
}

fn atomic_write(path: &Path, text: &str) -> Result<()> {
    crate::store::write_atomic(path, text.as_bytes())?;
    Ok(())
}

/// Create (no `id`) or update (existing `id`) a card. Returns the saved card.
pub fn save(repo: &Path, id: Option<&str>, title: &str, body: &str, status: Option<&str>, extra: &serde_json::Value) -> Result<Card> {
    if let Some(s) = status {
        if !STATUSES.contains(&s) {
            return Err(coded!("card.unknown_status", "unknown status {status:?}", status = s));
        }
    }
    // checked before anything is written: a bad agent must not leave a card file that no longer parses
    if let Some(a) = extra["agent"].as_str() {
        if !agents::ids().contains(&a) {
            return Err(coded!(
                "card.unknown_agent",
                "unknown agent {agent:?} (expected one of: {expected})",
                agent = a,
                expected = agents::ids().join(", ")
            ));
        }
    }
    let d = dir(repo);
    std::fs::create_dir_all(&d)?;
    exclude_kanban(repo)?;

    let (mut meta, path) = match id {
        Some(id) => {
            let c = list(repo)?
                .cards
                .into_iter()
                .find(|c| c.meta.id == id)
                .ok_or_else(|| coded!("card.not_found", "card {id} not found", id = id))?;
            (c.meta, c.path)
        }
        None => {
            let base = slug(title);
            let mut id = base.clone();
            let mut n = 2;
            while d.join(format!("{id}.md")).exists() {
                id = format!("{base}-{n}");
                n += 1;
            }
            let meta: Meta = serde_json::from_value(serde_json::json!({"id": id, "title": title}))?;
            let path = d.join(format!("{id}.md"));
            (meta, path)
        }
    };
    meta.title = title.to_string();
    if let Some(s) = status {
        meta.status = s.to_string();
    }
    if let Some(a) = extra["agent"].as_str() {
        meta.agent = a.to_string();
    }
    if let Some(p) = extra["priority"].as_i64() {
        meta.priority = p as i32;
    }
    if let Some(deps) = extra["depends_on"].as_array() {
        let deps: Vec<String> = deps.iter().filter_map(|v| v.as_str().map(String::from)).collect();
        if deps.iter().any(|x| !valid_id(x) || *x == meta.id) {
            return Err(coded!("card.bad_deps", "invalid dependency list"));
        }
        meta.depends_on = deps;
    }
    let front = serde_norway::to_string(&meta)?;
    atomic_write(&path, &format!("---\n{}---\n\n{}\n", front, body.trim_end()))?;
    let t = std::fs::read_to_string(&path)?;
    card::parse(&t, &path)
}

/// Status-only change: leaves the rest of the file byte-identical.
pub fn set_status(repo: &Path, id: &str, status: &str) -> Result<()> {
    if !STATUSES.contains(&status) {
        return Err(coded!("card.unknown_status", "unknown status {status:?}", status = status));
    }
    let c =
        list(repo)?.cards.into_iter().find(|c| c.meta.id == id).ok_or_else(|| coded!("card.not_found", "card {id} not found", id = id))?;
    card::set_status(&c.path, status)
}

pub fn delete(repo: &Path, id: &str) -> Result<()> {
    if !valid_id(id) {
        return Err(coded!("card.bad_id", "invalid card id"));
    }
    let c =
        list(repo)?.cards.into_iter().find(|c| c.meta.id == id).ok_or_else(|| coded!("card.not_found", "card {id} not found", id = id))?;
    std::fs::remove_file(c.path)?;
    Ok(())
}

/// Quiet time after the last file event before a change is reported (a card write is a few events).
const WATCH_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(150);

/// Watches `<repo>/.kanban` (cards written by the daemon, an orchestrator run, an agent or an editor) and calls `emit`
/// once per burst on a plain thread. `emit` returns false to stop. `done` runs when the thread ends (the folder was
/// removed, the stop above), so the caller can watch again later. Errors when the folder does not exist or cannot be watched.
pub fn watch(repo: &Path, emit: impl Fn() -> bool + Send + 'static, done: impl FnOnce() + Send + 'static) -> Result<()> {
    use notify::{RecursiveMode, Watcher};
    let d = dir(repo);
    let (tx, rx) = std::sync::mpsc::channel::<notify::Result<notify::Event>>();
    let mut w = notify::recommended_watcher(move |ev| {
        let _ = tx.send(ev);
    })
    .map_err(|e| coded!("watch.create_failed", "create watcher: {detail}", detail = e))?;
    w.watch(&d, RecursiveMode::NonRecursive).map_err(|e| coded!("watch.create_failed", "create watcher: {detail}", detail = e))?;
    std::thread::spawn(move || {
        let _w = w; // the watcher lives as long as this thread
        'outer: while let Ok(first) = rx.recv() {
            let mut gone = false;
            let mut seen = false;
            let mut take = |ev: notify::Result<notify::Event>| match ev {
                Ok(ev) if matches!(ev.kind, notify::EventKind::Access(_)) => {}
                Ok(ev) => {
                    seen = true;
                    gone |= matches!(ev.kind, notify::EventKind::Remove(_)) && ev.paths.iter().any(|p| p == &d);
                }
                // the OS dropped events: say "something changed"
                Err(_) => seen = true,
            };
            take(first);
            while let Ok(more) = rx.recv_timeout(WATCH_DEBOUNCE) {
                take(more);
            }
            if seen && !emit() {
                break 'outer;
            }
            if gone || !d.is_dir() {
                break;
            }
        }
        done();
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_carry_message_codes() {
        let repo = std::env::temp_dir().join(format!("axl-kanban-codes-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&repo);
        std::fs::create_dir_all(&repo).unwrap();
        axelstra_core::git::must(&repo, &["init", "-q"]).unwrap();
        let c = |e: anyhow::Error| axelstra_core::msg::find(&e).map(|c| c.code.clone()).unwrap_or_default();
        assert_eq!(c(save(&repo, None, "T", "b", Some("nope"), &serde_json::json!({})).unwrap_err()), "card.unknown_status");
        assert_eq!(c(save(&repo, None, "T", "b", None, &serde_json::json!({"agent": "rm -rf"})).unwrap_err()), "card.unknown_agent");
        assert_eq!(c(set_status(&repo, "ghost", "done").unwrap_err()), "card.not_found");
        assert_eq!(c(delete(&repo, "../x").unwrap_err()), "card.bad_id");
        // a broken file is listed with the code of its error
        std::fs::create_dir_all(repo.join(".kanban")).unwrap();
        std::fs::write(repo.join(".kanban/x.md"), "no frontmatter").unwrap();
        let b = list(&repo).unwrap().broken;
        assert_eq!(b[0].error_code.as_deref(), Some("card.no_frontmatter"));
        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn save_rejects_an_unknown_agent_and_writes_nothing() {
        let repo = std::env::temp_dir().join(format!("axl-kanban-agent-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&repo);
        std::fs::create_dir_all(&repo).unwrap();
        axelstra_core::git::must(&repo, &["init", "-q"]).unwrap();
        let e = save(&repo, None, "T", "b", None, &serde_json::json!({"agent": "rm -rf"})).unwrap_err().to_string();
        assert!(e.contains("unknown agent"), "{e}");
        assert!(!repo.join(".kanban").exists());
        for a in agents::ids() {
            save(&repo, None, "T", "b", None, &serde_json::json!({"agent": a})).unwrap();
        }
        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn an_unreadable_card_is_reported_not_fatal() {
        let repo = std::env::temp_dir().join(format!("axl-kanban-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&repo);
        std::fs::create_dir_all(repo.join(".kanban")).unwrap();
        // not UTF-8: read_to_string fails
        std::fs::write(repo.join(".kanban/bad.md"), [0xff, 0xfe, 0xfd]).unwrap();
        let b = list(&repo).unwrap();
        assert!(b.cards.is_empty());
        assert_eq!(b.broken.len(), 1);
        assert_eq!(b.broken[0].file, "bad.md");
        let _ = std::fs::remove_dir_all(&repo);
    }
}
