//! Projects (git repos the user added) and their worktrees. State is one small
//! JSON file; git itself is the source of truth for worktrees.

use crate::store::{read_json, write_atomic, LockRecover};
use anyhow::{bail, Context, Result};
use axelstra_core::git;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

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
    read_json(store).ok().flatten().unwrap_or_default()
}

/// Serializes read-modify-write of the projects file: two UI calls adding projects at once must not drop one.
static STORE_LOCK: Mutex<()> = Mutex::new(());

/// The list to modify and save back. Unlike `load`, a file that exists but cannot be read is an error, so an
/// empty list is never saved over projects we merely failed to read (a corrupt file was moved to `.bad` by then).
fn load_for_update(store: &Path) -> Result<Vec<Project>> {
    Ok(read_json(store).with_context(|| format!("could not read {}", store.display()))?.unwrap_or_default())
}

fn save(store: &Path, v: &[Project]) -> Result<()> {
    // temp file + rename: a crash never leaves a half-written list
    write_atomic(store, &serde_json::to_vec_pretty(v)?)?;
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
    let _g = STORE_LOCK.lock_recover();
    let mut v = load_for_update(store)?;
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
/// whose lockfile the project has, else the user's preferred one, else the first installed (bun, pnpm, yarn, npm).
pub fn package_managers(path: Option<&Path>, preferred: Option<&str>) -> (Vec<&'static str>, Option<&'static str>) {
    let installed: Vec<&'static str> = PACKAGE_MANAGERS.iter().copied().filter(|n| on_path(n)).collect();
    let locks =
        [("bun", "bun.lock"), ("bun", "bun.lockb"), ("pnpm", "pnpm-lock.yaml"), ("yarn", "yarn.lock"), ("npm", "package-lock.json")];
    let from_lock = path.and_then(|p| locks.iter().find(|(m, f)| installed.contains(m) && p.join(f).is_file()).map(|(m, _)| *m));
    let chosen = preferred.and_then(|m| installed.iter().copied().find(|i| *i == m));
    (installed.clone(), from_lock.or(chosen).or_else(|| installed.first().copied()))
}

pub fn set_setup(store: &Path, id: &str, setup: Option<String>) -> Result<()> {
    let _g = STORE_LOCK.lock_recover();
    let mut v = load_for_update(store)?;
    let p = v.iter_mut().find(|p| p.id == id).with_context(|| format!("unknown project {id}"))?;
    p.setup = setup.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    save(store, &v)
}

pub fn remove(store: &Path, id: &str) -> Result<()> {
    let _g = STORE_LOCK.lock_recover();
    let mut v = load_for_update(store)?;
    v.retain(|p| p.id != id);
    save(store, &v)
}

/// Only worktrees of registered projects may be inspected, never arbitrary paths.
/// A project whose folder is gone (or whose git fails) is skipped, not an error for every other project.
pub fn known_worktree(store: &Path, path: &str) -> Result<PathBuf> {
    let want = path.replace('\\', "/").to_ascii_lowercase();
    for p in load(store) {
        let Ok(list) = git::worktree_list(Path::new(&p.path)) else { continue };
        for w in list {
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
    Ok(create_at(p, name, "HEAD")?.0)
}

/// Like `worktree_create`, plus what `.worktreeinclude` copied into the new worktree (and what it left out).
pub fn worktree_create_report(p: &Project, name: &str) -> Result<(git::Worktree, crate::wtenv::Report)> {
    create_at(p, name, "HEAD")
}

/// Worktree on branch `ax/pr-<n>` holding the head of GitHub pull request `<n>`.
pub fn worktree_from_pr(p: &Project, number: u64) -> Result<git::Worktree> {
    let repo = Path::new(&p.path);
    git::must(repo, &["fetch", "origin", &format!("pull/{number}/head")])
        .with_context(|| format!("could not fetch pull request {number} from origin"))?;
    Ok(create_at(p, &format!("pr-{number}"), "FETCH_HEAD")?.0)
}

/// Folder of the worktree `name` of project `p`.
fn wt_dir(p: &Project, name: &str) -> Result<PathBuf> {
    let repo = Path::new(&p.path);
    Ok(repo.parent().context("repo has no parent directory")?.join(".axelstra-wt").join(format!("{}-{}", slug(&p.name), name)))
}

fn create_at(p: &Project, name: &str, base: &str) -> Result<(git::Worktree, crate::wtenv::Report)> {
    let n = slug(name);
    if n.is_empty() {
        bail!("worktree name is empty");
    }
    let repo = Path::new(&p.path);
    if !git::is_repo(repo) {
        bail!("este projeto ainda não é um repositório git: corre `git init` na pasta para poderes criar worktrees");
    }
    let dir = wt_dir(p, &n)?;
    if dir.exists() {
        bail!("{} already exists", dir.display());
    }
    std::fs::create_dir_all(dir.parent().unwrap())?;
    git::worktree_add(repo, &dir, &format!("ax/{n}"), base)?;
    // untracked config (.env...) the project asked for: a failure here never undoes the worktree
    let report = crate::wtenv::copy_into(repo, &dir);
    let wt = git::worktree_list(repo)?
        .into_iter()
        .find(|w| Path::new(&w.path).file_name() == dir.file_name())
        .context("worktree created but not listed")?;
    Ok((wt, report))
}

/// Renames a worktree: its folder and its branch move together to `ax/<name>`. Not for the main worktree, not while
/// an agent or terminal in it is busy (`busy`, known to the caller), and never over something that exists.
/// Returns the renamed worktree.
pub fn worktree_rename(p: &Project, path: &str, name: &str, busy: bool) -> Result<git::Worktree> {
    let repo = Path::new(&p.path);
    if !axelstra_core::card::valid_slug(name) {
        bail!("nome inválido: usa só letras, números, ponto, _ e - (sem começar por ponto)");
    }
    let norm = |s: &str| s.replace('\\', "/").trim_end_matches('/').to_ascii_lowercase();
    let wt = git::worktree_list(repo)?.into_iter().find(|w| norm(&w.path) == norm(path)).context("not a worktree of this project")?;
    if wt.is_main {
        bail!("cannot rename the main worktree");
    }
    if busy {
        bail!("há uma sessão ativa neste worktree: espera que fique inativa antes de mudar o nome");
    }
    let old_branch = wt.branch.clone().context("o worktree não tem ramo (detached HEAD)")?;
    let new_branch = format!("ax/{name}");
    let to = wt_dir(p, name)?;
    if to.exists() {
        bail!("{} already exists", to.display());
    }
    if git::run(repo, &["rev-parse", "--verify", "--quiet", &format!("refs/heads/{new_branch}")])?.ok {
        bail!("o ramo {new_branch} já existe");
    }
    git::forget_worktree_lists();
    git::must(repo, &["worktree", "move", &wt.path, &to.to_string_lossy()])?;
    if let Err(e) = git::must(repo, &["branch", "-m", &old_branch, &new_branch]) {
        // keep folder and branch in step: put the folder back
        let _ = git::must(repo, &["worktree", "move", &to.to_string_lossy(), &wt.path]);
        git::forget_worktree_lists();
        return Err(e);
    }
    git::forget_worktree_lists();
    git::worktree_list(repo)?.into_iter().find(|w| w.branch.as_deref() == Some(new_branch.as_str())).context("renamed worktree not listed")
}

pub fn worktree_remove(p: &Project, path: &str) -> Result<()> {
    let repo = Path::new(&p.path);
    let wt = git::worktree_list(repo)?.into_iter().find(|w| w.path.eq_ignore_ascii_case(path)).context("not a worktree of this project")?;
    if wt.is_main {
        bail!("cannot remove the main worktree");
    }
    git::worktree_remove(repo, Path::new(&wt.path))?;
    if let Some(b) = wt.branch {
        git::branch_delete(repo, &b)?;
    }
    Ok(())
}

const LOGO_CANDIDATES: [&str; 8] =
    ["logo.png", "logo.svg", "assets/logo.png", "assets/logo.svg", "docs/logo.png", ".github/logo.png", "icon.png", "assets/icon.png"];

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
        let (installed, suggested) = package_managers(Some(&dir), None);
        if let Some(s) = suggested {
            assert!(installed.contains(&s));
        }
        if installed.contains(&"npm") {
            assert_eq!(suggested, Some("npm"));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod store_tests {
    use super::*;

    fn root(tag: &str) -> PathBuf {
        let r = std::env::temp_dir().join(format!("axl-pstore-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&r);
        std::fs::create_dir_all(&r).unwrap();
        r
    }

    #[test]
    fn known_worktree_skips_a_project_whose_folder_is_gone() {
        let r = root("known");
        let real = r.join("real");
        std::fs::create_dir_all(&real).unwrap();
        let real_s = git::canon(&real).unwrap().to_string_lossy().to_string();
        let gone = r.join("gone").to_string_lossy().to_string();
        let store = r.join("projects.json");
        let list = vec![
            Project { id: "gone".into(), name: "gone".into(), path: gone, setup: None },
            Project { id: "real".into(), name: "real".into(), path: real_s.clone(), setup: None },
        ];
        save(&store, &list).unwrap();
        let wt = known_worktree(&store, &real_s).unwrap();
        let norm = |s: &str| s.to_ascii_lowercase().replace('\\', "/");
        assert_eq!(norm(&wt.to_string_lossy()), norm(&real_s));
        assert!(known_worktree(&store, "somewhere/else").is_err());
        let _ = std::fs::remove_dir_all(&r);
    }

    #[test]
    fn corrupt_projects_file_is_kept_aside_when_a_project_is_added() {
        let r = root("corrupt");
        let dir = r.join("proj");
        std::fs::create_dir_all(&dir).unwrap();
        let store = r.join("projects.json");
        std::fs::write(&store, "[{ not json").unwrap();
        assert!(load(&store).is_empty());
        add(&store, &dir.to_string_lossy()).unwrap();
        assert_eq!(std::fs::read_to_string(crate::store::bad_path(&store)).unwrap(), "[{ not json");
        assert_eq!(load(&store).len(), 1);
        let _ = std::fs::remove_dir_all(&r);
    }

    #[test]
    fn an_unreadable_projects_file_is_never_overwritten_by_an_update() {
        let r = root("unreadable");
        let store = r.join("projects.json");
        // a folder where the file should be: it exists but cannot be read
        std::fs::create_dir_all(&store).unwrap();
        assert!(load(&store).is_empty());
        assert!(remove(&store, "x").is_err());
        assert!(store.is_dir());
        let _ = std::fs::remove_dir_all(&r);
    }

    #[test]
    fn concurrent_adds_all_survive() {
        let r = root("concurrent");
        let store = r.join("projects.json");
        let threads: Vec<_> = (0..8)
            .map(|i| {
                let (dir, store) = (r.join(format!("p{i}")), store.clone());
                std::fs::create_dir_all(&dir).unwrap();
                std::thread::spawn(move || add(&store, &dir.to_string_lossy()).unwrap())
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        assert_eq!(load(&store).len(), 8);
        let _ = std::fs::remove_dir_all(&r);
    }
}

#[cfg(test)]
mod rename_tests {
    use super::*;

    fn setup(tag: &str) -> (PathBuf, Project) {
        let root = std::env::temp_dir().join(format!("axl-rename-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let repo = root.join("proj");
        std::fs::create_dir_all(&repo).unwrap();
        git::must(&repo, &["init", "-q", "-b", "main"]).unwrap();
        std::fs::write(repo.join("a.txt"), "1").unwrap();
        git::must(&repo, &["add", "."]).unwrap();
        git::must(&repo, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qm", "init"]).unwrap();
        let path = git::canon(&repo).unwrap().to_string_lossy().to_string();
        (root, Project { id: "p".into(), name: "proj".into(), path, setup: None })
    }

    #[test]
    fn rename_moves_folder_and_branch_together() {
        let (root, p) = setup("ok");
        let w = worktree_create(&p, "old").unwrap();
        let r = worktree_rename(&p, &w.path, "fresh", false).unwrap();
        assert_eq!(r.branch.as_deref(), Some("ax/fresh"));
        assert!(Path::new(&r.path).ends_with("proj-fresh"));
        assert!(Path::new(&r.path).join("a.txt").is_file());
        assert!(!Path::new(&w.path).exists());
        assert!(!git::run(Path::new(&p.path), &["rev-parse", "--verify", "--quiet", "refs/heads/ax/old"]).unwrap().ok);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn rename_refuses_bad_cases() {
        let (root, p) = setup("no");
        let a = worktree_create(&p, "a").unwrap();
        let b = worktree_create(&p, "b").unwrap();
        let err = |r: Result<git::Worktree>| r.unwrap_err().to_string();
        assert!(err(worktree_rename(&p, &a.path, "b", false)).contains("already exists"));
        assert!(err(worktree_rename(&p, &a.path, "../x", false)).contains("inválido"));
        assert!(err(worktree_rename(&p, &a.path, ".hidden", false)).contains("inválido"));
        assert!(err(worktree_rename(&p, &a.path, "", false)).contains("inválido"));
        assert!(err(worktree_rename(&p, &a.path, "busy", true)).contains("sessão ativa"));
        assert!(err(worktree_rename(&p, &p.path, "main2", false)).contains("main worktree"));
        assert!(err(worktree_rename(&p, "C:/nowhere", "z", false)).contains("not a worktree"));
        // a branch of that name without a folder blocks it too
        git::must(Path::new(&p.path), &["branch", "ax/taken"]).unwrap();
        assert!(err(worktree_rename(&p, &b.path, "taken", false)).contains("já existe"));
        assert!(Path::new(&a.path).exists() && Path::new(&b.path).exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn create_copies_worktreeinclude_files() {
        let (root, p) = setup("env");
        let repo = Path::new(&p.path);
        std::fs::write(repo.join(".gitignore"), ".env\n").unwrap();
        std::fs::write(repo.join(".env"), "KEY=1").unwrap();
        std::fs::write(repo.join(crate::wtenv::INCLUDE_FILE), ".env\n").unwrap();
        git::must(repo, &["add", ".gitignore", ".worktreeinclude"]).unwrap();
        git::must(repo, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qm", "env"]).unwrap();
        let (w, rep) = worktree_create_report(&p, "withenv").unwrap();
        assert_eq!(rep.copied, vec![".env"]);
        assert_eq!(std::fs::read_to_string(Path::new(&w.path).join(".env")).unwrap(), "KEY=1");
        let _ = std::fs::remove_dir_all(&root);
    }
}
