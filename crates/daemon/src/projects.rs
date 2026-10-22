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
    /// command run in a terminal of every new worktree (e.g. `bun install`)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub setup: Option<String>,
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

/// Accented letters become their plain form ("Otimização" -> "Otimizacao"); anything else outside
/// `[A-Za-z0-9_-]` becomes `-`, and runs of `-` collapse into one.
fn slug(s: &str) -> String {
    let plain = |c: char| match c {
        'á' | 'à' | 'â' | 'ã' | 'ä' | 'å' => 'a',
        'Á' | 'À' | 'Â' | 'Ã' | 'Ä' | 'Å' => 'A',
        'é' | 'è' | 'ê' | 'ë' => 'e',
        'É' | 'È' | 'Ê' | 'Ë' => 'E',
        'í' | 'ì' | 'î' | 'ï' => 'i',
        'Í' | 'Ì' | 'Î' | 'Ï' => 'I',
        'ó' | 'ò' | 'ô' | 'õ' | 'ö' => 'o',
        'Ó' | 'Ò' | 'Ô' | 'Õ' | 'Ö' => 'O',
        'ú' | 'ù' | 'û' | 'ü' => 'u',
        'Ú' | 'Ù' | 'Û' | 'Ü' => 'U',
        'ç' => 'c',
        'Ç' => 'C',
        'ñ' => 'n',
        'Ñ' => 'N',
        c => c,
    };
    let mut t = String::new();
    for c in s.chars().map(plain) {
        let c = if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' };
        if c == '-' && t.ends_with('-') {
            continue;
        }
        t.push(c);
    }
    t.trim_matches('-').to_string()
}

pub fn add(store: &Path, path: &str) -> Result<Project> {
    // a folder outside any repository is accepted as it is: the project can start from prompts and `git init` later
    let root = match git::repo_root(Path::new(path)) {
        Ok(r) => r,
        Err(_) if Path::new(path).is_dir() => git::canon(Path::new(path))?,
        Err(e) => return Err(e).with_context(|| format!("{path} is not a folder")),
    };
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
    let p = Project { id, name, path: root_s, setup: None };
    v.push(p.clone());
    save(store, &v)?;
    Ok(p)
}

const PACKAGE_MANAGERS: [&str; 4] = ["bun", "pnpm", "yarn", "npm"];

fn on_path(name: &str) -> bool {
    let exts: Vec<String> = if cfg!(windows) {
        std::env::var("PATHEXT").unwrap_or(".EXE;.CMD;.BAT".into()).split(';').map(|e| e.to_ascii_lowercase()).collect()
    } else {
        vec![String::new()]
    };
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|d| exts.iter().any(|e| d.join(format!("{name}{e}")).is_file())))
        .unwrap_or(false)
}

/// Package managers installed on this machine, and the one to suggest for the project at `path`: the manager
/// whose lockfile the project has, else the first installed one (bun, pnpm, yarn, npm in that order).
pub fn package_managers(path: Option<&Path>) -> (Vec<&'static str>, Option<&'static str>) {
    let installed: Vec<&'static str> = PACKAGE_MANAGERS.iter().copied().filter(|n| on_path(n)).collect();
    let locks = [("bun", "bun.lock"), ("bun", "bun.lockb"), ("pnpm", "pnpm-lock.yaml"), ("yarn", "yarn.lock"), ("npm", "package-lock.json")];
    let from_lock = path.and_then(|p| locks.iter().find(|(m, f)| installed.contains(m) && p.join(f).is_file()).map(|(m, _)| *m));
    (installed.clone(), from_lock.or_else(|| installed.first().copied()))
}

pub fn set_setup(store: &Path, id: &str, setup: Option<String>) -> Result<()> {
    let mut v = load(store);
    let p = v.iter_mut().find(|p| p.id == id).with_context(|| format!("unknown project {id}"))?;
    p.setup = setup.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    save(store, &v)
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
    create_at(p, name, "HEAD")
}

/// Worktree on branch `ax/pr-<n>` holding the head of GitHub pull request `<n>`.
pub fn worktree_from_pr(p: &Project, number: u64) -> Result<git::Worktree> {
    let repo = Path::new(&p.path);
    git::must(repo, &["fetch", "origin", &format!("pull/{number}/head")])
        .with_context(|| format!("could not fetch pull request {number} from origin"))?;
    create_at(p, &format!("pr-{number}"), "FETCH_HEAD")
}

fn create_at(p: &Project, name: &str, base: &str) -> Result<git::Worktree> {
    let n = slug(name);
    if n.is_empty() {
        bail!("worktree name is empty");
    }
    let repo = Path::new(&p.path);
    if !git::is_repo(repo) {
        bail!("este projeto ainda não é um repositório git: corre `git init` na pasta para poderes criar worktrees");
    }
    let dir = repo
        .parent()
        .context("repo has no parent directory")?
        .join(".axelstra-wt")
        .join(format!("{}-{}", slug(&p.name), n));
    if dir.exists() {
        bail!("{} already exists", dir.display());
    }
    std::fs::create_dir_all(dir.parent().unwrap())?;
    git::worktree_add(repo, &dir, &format!("ax/{n}"), base)?;
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

#[cfg(test)]
mod slug_tests {
    use super::slug;

    #[test]
    fn accents_become_plain_letters() {
        assert_eq!(slug("Otimização"), "Otimizacao");
        assert_eq!(slug("Correção de ação  rápida!"), "Correcao-de-acao-rapida");
        assert_eq!(slug("--a_b--"), "a_b");
    }
}

#[cfg(test)]
mod plain_folder_tests {
    use super::*;

    #[test]
    fn a_folder_without_git_is_a_project_with_one_main_worktree() {
        let root = std::env::temp_dir().join(format!("axl-plain-{}", std::process::id()));
        let dir = root.join("novo-projeto");
        std::fs::create_dir_all(&dir).unwrap();
        let store = root.join("projects.json");
        let p = add(&store, &dir.to_string_lossy()).unwrap();
        assert_eq!(p.name, "novo-projeto");
        let wts = git::worktree_list(Path::new(&p.path)).unwrap();
        assert_eq!(wts.len(), 1);
        assert!(wts[0].is_main);
        let err = worktree_create(&p, "x").unwrap_err().to_string();
        assert!(err.contains("git init"), "{err}");
        let _ = std::fs::remove_dir_all(&root);
    }
}

#[cfg(test)]
mod package_manager_tests {
    use super::package_managers;

    #[test]
    fn suggestion_is_installed_and_follows_the_lockfile() {
        let dir = std::env::temp_dir().join(format!("axl-pm-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("package-lock.json"), "{}").unwrap();
        let (installed, suggested) = package_managers(Some(&dir));
        if let Some(s) = suggested {
            assert!(installed.contains(&s));
        }
        if installed.contains(&"npm") {
            assert_eq!(suggested, Some("npm"));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
