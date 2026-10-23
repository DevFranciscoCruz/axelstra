//! Names and task files of agents. Every agent gets a name ("Tarefa N", counted per project) and a markdown file with
//! its goal and a "Relatorio" section. The files live in the app data directory, outside the repository, in one
//! folder per project; `axelstra note` and `axelstra done` append to them, the app reads and exports them.

use anyhow::{bail, Result};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::store::LockRecover;

/// Longest text one append may add, and largest size a task file may grow to.
const MAX_APPEND: usize = 64 * 1024;
const MAX_FILE: u64 = 1024 * 1024;

/// Last number handed out per project: a name is never reused while the daemon lives.
static COUNTERS: Mutex<Option<HashMap<String, u32>>> = Mutex::new(None);

/// Folder of all task files.
pub fn root(data_dir: &Path) -> PathBuf {
    data_dir.join("notes")
}

/// A name that is safe as one file or folder name on every platform: no separators, no characters Windows refuses,
/// no reserved device names, no dots or spaces at the ends, at most 80 characters.
pub fn safe_name(s: &str) -> String {
    let cleaned: String = s.chars().map(|c| if c.is_control() || "<>:\"/\\|?*".contains(c) { '_' } else { c }).collect();
    let trimmed: String = cleaned.trim().trim_matches('.').trim().chars().take(80).collect();
    let trimmed = trimmed.trim().to_string();
    if trimmed.is_empty() {
        return "agente".into();
    }
    let stem = trimmed.split('.').next().unwrap_or_default().to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((stem.starts_with("COM") || stem.starts_with("LPT")) && stem.len() == 4 && stem.as_bytes()[3].is_ascii_digit());
    if reserved {
        format!("_{trimmed}")
    } else {
        trimmed
    }
}

/// "Tarefa 7" -> 7
pub fn default_number(label: &str) -> Option<u32> {
    label.strip_prefix("Tarefa ")?.trim().parse().ok()
}

fn bump(project: &str, n: u32) {
    let mut g = COUNTERS.lock_recover();
    let e = g.get_or_insert_with(HashMap::new).entry(project.to_string()).or_insert(0);
    *e = (*e).max(n);
}

/// Next free number of a project: above everything handed out so far, and with no file of that name yet.
fn next_number(dir: &Path, project: &str) -> u32 {
    let mut n = COUNTERS.lock_recover().get_or_insert_with(HashMap::new).get(project).copied().unwrap_or(0);
    loop {
        n += 1;
        if !dir.join(format!("Tarefa {n}.md")).exists() {
            break;
        }
    }
    bump(project, n);
    n
}

/// Numbering of a project starts again: the counter drops to 0 and the task files of earlier agents move to
/// `arquivo/` (kept, never deleted) so their names are free.
fn restart(dir: &Path, project: &str) {
    COUNTERS.lock_recover().get_or_insert_with(HashMap::new).remove(project);
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let old: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.is_file() && p.extension().is_some_and(|x| x == "md")).collect();
    if old.is_empty() {
        return;
    }
    let arch = dir.join("arquivo");
    if std::fs::create_dir_all(&arch).is_err() {
        return;
    }
    let tag = chrono::Local::now().format("%Y%m%d-%H%M%S");
    for p in old {
        let Some(stem) = p.file_stem().map(|s| s.to_string_lossy().into_owned()) else { continue };
        let _ = std::fs::rename(&p, arch.join(format!("{stem} {tag}.md")));
    }
}

fn stamp() -> String {
    chrono::Local::now().format("%Y-%m-%d %H:%M").to_string()
}

fn template(name: &str, agent: &str, worktree: &str, goal: Option<&str>) -> String {
    let goal = goal.map(str::trim).filter(|g| !g.is_empty()).unwrap_or("_(sem objetivo definido)_");
    format!("# {name}\n\n- Agente: {agent}\n- Worktree: {worktree}\n- Criado: {}\n\n## Objetivo\n\n{goal}\n\n## Relatorio\n", stamp())
}

/// Creates the file for `name` in the project folder, never over an existing one (`name (2).md`...).
fn create(dir: &Path, name: &str, body: &str) -> Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let base = safe_name(name);
    for i in 1..1000 {
        let file = if i == 1 { format!("{base}.md") } else { format!("{base} ({i}).md") };
        let path = dir.join(file);
        // create_new: two agents starting at once cannot pick the same file
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut f) => {
                std::io::Write::write_all(&mut f, body.as_bytes())?;
                return Ok(path);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.into()),
        }
    }
    bail!("too many task files named {base}")
}

/// True when `p` is an existing markdown file inside the notes folder. A path that came from a client is only
/// trusted after this.
pub fn inside_root(root: &Path, p: &Path) -> bool {
    let (Ok(r), Ok(f)) = (root.canonicalize(), p.canonicalize()) else { return false };
    f.starts_with(&r) && f.is_file() && f.extension().is_some_and(|e| e == "md")
}

/// Called when a session is spawned. For an agent it settles the name and the task file and records the file in
/// `meta["note"]`; returns the label to use. A label other than the agent's own name (renamed, or restored) is kept.
/// `goal` is the prompt the agent starts with, if any.
/// `alone` is true when the project has no other running agent: a new name then starts again at "Tarefa 1" and the
/// finished tasks' files move to `arquivo/`. The main worktree is no task: its agent gets no name and no file.
pub fn on_spawn(root: &Path, label: Option<&str>, meta: &mut Option<Value>, goal: Option<&str>, alone: bool) -> Option<String> {
    let kept = label.map(str::to_string);
    let Some(m) = meta.as_mut().filter(|m| m["kind"] == "agent" && m["main"] != true) else { return kept };
    let project = safe_name(m["project"].as_str().unwrap_or("geral"));
    let agent = m["agent"].as_str().unwrap_or("agent").to_string();
    let dir = root.join(&project);
    let name = match label.map(str::trim).filter(|l| !l.is_empty() && *l != agent) {
        Some(l) => {
            if let Some(n) = default_number(l) {
                bump(&project, n);
            }
            l.to_string()
        }
        None => {
            if alone {
                restart(&dir, &project);
            }
            format!("Tarefa {}", next_number(&dir, &project))
        }
    };
    // a restored agent comes back with its file; the path is only believed when it is really one of ours
    let known = m["note"].as_str().map(PathBuf::from).filter(|p| inside_root(root, p));
    if known.is_none() {
        let wt = m["worktree"].as_str().unwrap_or_default();
        match create(&dir, &name, &template(&name, &agent, wt, goal)) {
            Ok(p) => m["note"] = json!(p.to_string_lossy()),
            Err(e) => {
                eprintln!("task file for {name}: {e}");
                m.as_object_mut().map(|o| o.remove("note"));
            }
        }
    }
    Some(name)
}

/// Adds a dated entry at the end of the file (the Relatorio section is the last one).
pub fn append(path: &Path, title: &str, text: &str) -> Result<()> {
    let text = text.trim();
    if text.is_empty() {
        bail!("nothing to write");
    }
    if text.len() > MAX_APPEND {
        bail!("text too long (max {} KB)", MAX_APPEND / 1024);
    }
    if std::fs::metadata(path)?.len() > MAX_FILE {
        bail!("the task file is full");
    }
    let mut f = std::fs::OpenOptions::new().append(true).open(path)?;
    std::io::Write::write_all(&mut f, format!("\n### {} - {title}\n\n{text}\n", stamp()).as_bytes())?;
    Ok(())
}

/// "[ok] text" as sent by `axelstra done` -> ("ok", "text"); anything else counts as a plain report.
pub fn split_status(body: &str) -> (&str, &str) {
    for s in ["ok", "failed", "blocked"] {
        if let Some(rest) = body.strip_prefix(&format!("[{s}]")) {
            return (s, rest.trim_start());
        }
    }
    ("ok", body)
}

pub fn read(path: &Path) -> Result<String> {
    if std::fs::metadata(path)?.len() > MAX_FILE * 2 {
        bail!("the task file is too large to show");
    }
    Ok(std::fs::read_to_string(path)?)
}

/// One task file of a worktree, as the empty-state list shows it.
#[derive(Debug, Clone, serde::Serialize, PartialEq)]
pub struct NoteInfo {
    /// file name without `.md`: also the name passed back to read it
    pub name: String,
    /// "ok" once the agent reported it finished, "nota" for notes only, "aberto" when nothing was written yet
    pub status: String,
    /// the first lines of the Relatorio section
    pub excerpt: String,
    /// seconds since the epoch of the last change
    pub modified: u64,
}

fn header_value<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    text.lines().take(8).find_map(|l| l.strip_prefix(&format!("- {key}: ")).map(str::trim))
}

/// Status and first lines of the Relatorio section of a task file.
fn summarize(text: &str) -> (String, String) {
    let body = text.split_once("\n## Relatorio").map(|(_, r)| r).unwrap_or_default();
    let status = if body.contains("- ok\n") {
        "ok"
    } else if body.contains("### ") {
        "nota"
    } else {
        "aberto"
    };
    let lines: Vec<&str> = body.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).take(3).collect();
    let mut excerpt = lines.join(" ");
    if excerpt.chars().count() > 240 {
        excerpt = excerpt.chars().take(240).collect::<String>() + "…";
    }
    (status.into(), excerpt)
}

/// The task files of a project whose header names `worktree` (newest first). Files that cannot be read are skipped.
pub fn for_worktree(root: &Path, project: &str, worktree: &str) -> Vec<NoteInfo> {
    let want = |s: &str| s.replace('\\', "/").trim_end_matches('/').to_ascii_lowercase();
    let want_wt = want(worktree);
    let Ok(rd) = std::fs::read_dir(root.join(safe_name(project))) else { return vec![] };
    let mut out: Vec<NoteInfo> = rd
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "md"))
        .filter_map(|e| {
            let path = e.path();
            let text = read(&path).ok()?;
            if want(header_value(&text, "Worktree")?) != want_wt {
                return None;
            }
            let (status, excerpt) = summarize(&text);
            let modified = e.metadata().ok()?.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
            Some(NoteInfo { name: path.file_stem()?.to_string_lossy().into_owned(), status, excerpt, modified })
        })
        .collect();
    out.sort_by(|a, b| b.modified.cmp(&a.modified).then_with(|| a.name.cmp(&b.name)));
    out
}

/// The task file `name` of `project`, if it is one of ours.
pub fn file_of(root: &Path, project: &str, name: &str) -> Option<PathBuf> {
    let p = root.join(safe_name(project)).join(format!("{}.md", safe_name(name)));
    inside_root(root, &p).then_some(p)
}

/// Copies the task file into `worktree/.axelstra/`. Never replaces a file already there unless `overwrite`.
/// The caller checked that `worktree` is a known worktree.
pub fn export(note: &Path, worktree: &Path, overwrite: bool) -> Result<PathBuf> {
    let name = note.file_name().ok_or_else(|| anyhow::anyhow!("bad task file"))?;
    let dir = worktree.join(".axelstra");
    std::fs::create_dir_all(&dir)?;
    let target = dir.join(name);
    if target.exists() && !overwrite {
        bail!("{} already exists; export again with overwrite to replace it", target.display());
    }
    std::fs::copy(note, &target)?;
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("axl-notes-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn agent_meta(project: &str) -> Option<Value> {
        Some(json!({"project": project, "worktree": "C:/x/wt", "kind": "agent", "agent": "claude"}))
    }

    #[test]
    fn safe_name_blocks_traversal_and_reserved_names() {
        assert_eq!(safe_name("../../etc/passwd"), "_.._etc_passwd");
        assert_eq!(safe_name("a\\b:c*d"), "a_b_c_d");
        assert_eq!(safe_name("CON"), "_CON");
        assert_eq!(safe_name("com1.txt"), "_com1.txt");
        assert_eq!(safe_name("  .. "), "agente");
        assert_eq!(safe_name(""), "agente");
        assert_eq!(safe_name("Tarefa 3"), "Tarefa 3");
        assert_eq!(safe_name(&"x".repeat(300)).len(), 80);
        assert!(!safe_name("a/b").contains('/'));
    }

    #[test]
    fn default_names_count_per_project_and_are_not_reused() {
        let root = dir("count");
        let mut m = agent_meta("p-count");
        assert_eq!(on_spawn(&root, Some("claude"), &mut m, None, false).as_deref(), Some("Tarefa 1"));
        let mut m2 = agent_meta("p-count");
        assert_eq!(on_spawn(&root, None, &mut m2, None, false).as_deref(), Some("Tarefa 2"));
        let mut other = agent_meta("p-count-other");
        assert_eq!(on_spawn(&root, Some("claude"), &mut other, None, false).as_deref(), Some("Tarefa 1"));
        // the first agent is renamed and its file stays: the number still is not reused
        let mut m3 = agent_meta("p-count");
        assert_eq!(on_spawn(&root, Some("claude"), &mut m3, None, false).as_deref(), Some("Tarefa 3"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn restored_agent_keeps_label_and_file() {
        let root = dir("restore");
        let mut m = agent_meta("p-restore");
        let name = on_spawn(&root, Some("claude"), &mut m, Some("faz X"), false).unwrap();
        let note = m.as_ref().unwrap()["note"].as_str().unwrap().to_string();
        let mut back = agent_meta("p-restore");
        back.as_mut().unwrap()["note"] = json!(note);
        assert_eq!(on_spawn(&root, Some(&name), &mut back, None, false), Some(name));
        assert_eq!(back.unwrap()["note"], json!(note));
        assert_eq!(std::fs::read_dir(root.join("p-restore")).unwrap().count(), 1);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_client_cannot_point_the_note_outside_the_folder() {
        let root = dir("evil");
        let outside = dir("evil-out").join("secret.md");
        std::fs::write(&outside, "x").unwrap();
        let mut m = agent_meta("p-evil");
        m.as_mut().unwrap()["note"] = json!(outside.to_string_lossy());
        on_spawn(&root, Some("claude"), &mut m, None, false);
        let note = PathBuf::from(m.unwrap()["note"].as_str().unwrap());
        assert!(inside_root(&root, &note));
        assert_ne!(note, outside);
        assert!(!inside_root(&root, &outside));
        assert!(!inside_root(&root, &root.join("p-evil").join("..").join("..").join("nothing.md")));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn terminals_get_no_file_and_keep_their_label() {
        let root = dir("term");
        let mut m = Some(json!({"project": "p", "kind": "terminal"}));
        assert_eq!(on_spawn(&root, Some("terminal"), &mut m, None, false).as_deref(), Some("terminal"));
        assert!(m.unwrap().get("note").is_none());
        assert!(!root.exists() || std::fs::read_dir(&root).unwrap().count() == 0);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn file_has_goal_and_report_section_and_appends() {
        let root = dir("body");
        let mut m = agent_meta("p-body");
        on_spawn(&root, Some("claude"), &mut m, Some("corrigir o <login> & testar \"tudo\""), false);
        let p = PathBuf::from(m.unwrap()["note"].as_str().unwrap());
        let t = read(&p).unwrap();
        assert!(t.starts_with("# Tarefa "), "{t}");
        assert!(t.contains("## Objetivo\n\ncorrigir o <login> & testar \"tudo\""), "{t}");
        assert!(t.trim_end().ends_with("## Relatorio"), "{t}");
        append(&p, "ok", "feito").unwrap();
        append(&p, "nota", "mais uma").unwrap();
        let t = read(&p).unwrap();
        assert!(t.contains("- ok\n\nfeito\n") && t.contains("- nota\n\nmais uma\n"), "{t}");
        assert!(append(&p, "nota", "   ").is_err());
        assert!(append(&p, "nota", &"x".repeat(MAX_APPEND + 1)).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn done_body_is_split_into_status_and_text() {
        assert_eq!(split_status("[failed] não deu"), ("failed", "não deu"));
        assert_eq!(split_status("[blocked]x"), ("blocked", "x"));
        assert_eq!(split_status("sem estado"), ("ok", "sem estado"));
    }

    #[test]
    fn export_copies_and_refuses_to_overwrite() {
        let root = dir("export");
        let wt = dir("export-wt");
        let mut m = agent_meta("p-export");
        on_spawn(&root, Some("claude"), &mut m, None, false);
        let note = PathBuf::from(m.unwrap()["note"].as_str().unwrap());
        let out = export(&note, &wt, false).unwrap();
        assert_eq!(out.parent().unwrap(), wt.join(".axelstra"));
        assert_eq!(read(&out).unwrap(), read(&note).unwrap());
        assert!(export(&note, &wt, false).is_err());
        append(&note, "nota", "novo").unwrap();
        export(&note, &wt, true).unwrap();
        assert!(read(&out).unwrap().contains("novo"));
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&wt);
    }

    #[test]
    fn notes_are_found_by_the_worktree_in_their_header() {
        let root = dir("bywt");
        let mut a = Some(json!({"project": "p-bywt", "worktree": "C:/x/Wt-A", "kind": "agent", "agent": "claude"}));
        let mut b = Some(json!({"project": "p-bywt", "worktree": "C:/x/wt-b", "kind": "agent", "agent": "claude"}));
        on_spawn(&root, Some("claude"), &mut a, Some("faz A"), false);
        on_spawn(&root, Some("claude"), &mut b, None, false);
        let pa = PathBuf::from(a.unwrap()["note"].as_str().unwrap());
        assert_eq!(for_worktree(&root, "p-bywt", "c:\\x\\wt-a/")[0].status, "aberto");
        append(&pa, "ok", "feito\ncom detalhe").unwrap();
        let found = for_worktree(&root, "p-bywt", "C:/x/wt-a");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].status.as_str(), found[0].excerpt.as_str()), ("ok", "feito com detalhe"));
        assert_eq!(for_worktree(&root, "p-bywt", "C:/x/other").len(), 0);
        assert_eq!(for_worktree(&root, "no-such-project", "C:/x/wt-a").len(), 0);
        assert!(file_of(&root, "p-bywt", &found[0].name).is_some());
        assert!(file_of(&root, "p-bywt", "../../x").is_none());
        let _ = std::fs::remove_dir_all(&root);
    }
}
