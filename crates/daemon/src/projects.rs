//! Projects (git repos the user added) and their worktrees. State is one small
//! JSON file; git itself is the source of truth for worktrees.

use anyhow::{bail, Context, Result};
use axelstra_core::git;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub path: String,
}

pub fn store_path() -> PathBuf {
    crate::proto::token_path().with_file_name("projects.json")
}

pub fn load(store: &Path) -> Vec<Project> {
    std::fs::read_to_string(store).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

fn save(store: &Path, v: &[Project]) -> Result<()> {
    std::fs::create_dir_all(store.parent().unwrap())?;
    // temp file + rename: a crash never leaves a half-written list
    let tmp = store.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(v)?)?;
    std::fs::rename(&tmp, store)?;
    Ok(())
}

fn slug(s: &str) -> String {
    let t: String = s
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
        .collect();
    t.trim_matches('-').to_string()
}

pub fn add(store: &Path, path: &str) -> Result<Project> {
    let root = git::repo_root(Path::new(path)).with_context(|| format!("{path} is not inside a git repository"))?;
    let root_s = root.to_string_lossy().to_string();
    let mut v = load(store);
    if let Some(p) = v.iter().find(|p| p.path.eq_ignore_ascii_case(&root_s)) {
        return Ok(p.clone());
    }
    let name = root.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "project".into());
    let mut id = slug(&name);
    while v.iter().any(|p| p.id == id) {
        id.push_str("-2");
    }
    let p = Project { id, name, path: root_s };
    v.push(p.clone());
    save(store, &v)?;
    Ok(p)
}

pub fn remove(store: &Path, id: &str) -> Result<()> {
    let mut v = load(store);
    v.retain(|p| p.id != id);
    save(store, &v)
}

/// Only worktrees of registered projects may be inspected, never arbitrary paths.
pub fn known_worktree(store: &Path, path: &str) -> Result<PathBuf> {
    let want = path.replace('\\', "/").to_ascii_lowercase();
    for p in load(store) {
        for w in git::worktree_list(Path::new(&p.path))? {
            if w.path.replace('\\', "/").to_ascii_lowercase() == want {
                return Ok(PathBuf::from(w.path));
            }
        }
    }
    bail!("not a worktree of a registered project")
}

pub fn find(store: &Path, id: &str) -> Result<Project> {
    load(store).into_iter().find(|p| p.id == id).with_context(|| format!("unknown project {id}"))
}

/// New worktree on a new branch `ax/<name>` from the repo's current HEAD.
/// Lives next to the repo in `.axelstra-wt/` so paths stay short.
pub fn worktree_create(p: &Project, name: &str) -> Result<git::Worktree> {
    let n = slug(name);
    if n.is_empty() {
        bail!("worktree name is empty");
    }
    let repo = Path::new(&p.path);
    let dir = repo
        .parent()
        .context("repo has no parent directory")?
        .join(".axelstra-wt")
        .join(format!("{}-{}", slug(&p.name), n));
    if dir.exists() {
        bail!("{} already exists", dir.display());
    }
    std::fs::create_dir_all(dir.parent().unwrap())?;
    git::worktree_add(repo, &dir, &format!("ax/{n}"), "HEAD")?;
    git::worktree_list(repo)?
        .into_iter()
        .find(|w| Path::new(&w.path).file_name() == dir.file_name())
        .context("worktree created but not listed")
}

pub fn worktree_remove(p: &Project, path: &str) -> Result<()> {
    let repo = Path::new(&p.path);
    let wt = git::worktree_list(repo)?
        .into_iter()
        .find(|w| w.path.eq_ignore_ascii_case(path))
        .context("not a worktree of this project")?;
    if wt.is_main {
        bail!("cannot remove the main worktree");
    }
    git::worktree_remove(repo, Path::new(&wt.path))?;
    if let Some(b) = wt.branch {
        git::branch_delete(repo, &b)?;
    }
    Ok(())
}

const LOGO_CANDIDATES: [&str; 8] = [
    "logo.png", "logo.svg", "assets/logo.png", "assets/logo.svg", "docs/logo.png", ".github/logo.png", "icon.png", "assets/icon.png",
];

/// A logo shipped in the repo itself, as a data URL. None when there is none
/// (the UI then falls back to the git host's avatar).
pub fn logo_data_url(repo: &Path) -> Option<String> {
    use base64::Engine;
    for rel in LOGO_CANDIDATES {
        let p = repo.join(rel);
        let Ok(md) = std::fs::metadata(&p) else { continue };
        if !md.is_file() || md.len() > 512 * 1024 {
            continue;
        }
        let Ok(bytes) = std::fs::read(&p) else { continue };
        let mime = if rel.ends_with(".svg") { "image/svg+xml" } else { "image/png" };
        return Some(format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes)));
    }
    None
}
