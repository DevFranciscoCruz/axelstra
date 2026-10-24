//! Projects (git repos the user added) and their worktrees. State is one small
//! JSON file; git itself is the source of truth for worktrees.

use crate::store::{read_json, write_json, LockRecover};
use anyhow::Result;
use axelstra_core::{coded, git};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
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
    /// worktree path -> path of its parent worktree (both normalized with `norm_path`): worktrees of one theme are
    /// shown together under the parent. Entries of worktrees that no longer exist are harmless and ignored.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub parents: BTreeMap<String, String>,
}

/// The form of a worktree path used as a key: the shared comparison key (forward slashes, no trailing slash, no
/// verbatim `\\?\` prefix, lowercase only where the file system ignores case).
pub fn norm_path(s: &str) -> String {
    axelstra_core::paths::norm(s)
}

pub fn store_path() -> PathBuf {
    crate::proto::token_path().with_file_name("projects.json")
}

/// The saved projects. A file that exists but cannot be read is an error, never silently an empty list; a corrupt
/// one was already moved aside by `read_json`, which recorded a notice.
pub fn try_load(store: &Path) -> Result<Vec<Project>> {
    Ok(read_json(store)
        .map_err(|e| coded!("project.read_failed", "could not read {path}: {detail}", path = store.display(), detail = e))?
        .unwrap_or_default())
}

/// `try_load` for read-only callers that cannot report: a read error is logged and recorded as a notice
/// (`store::take_notices`) and the list is empty for that call only; nothing is saved back from it.
pub fn load(store: &Path) -> Vec<Project> {
    try_load(store).unwrap_or_else(|e| {
        let detail = format!("{e:#}");
        crate::store::notice_coded(axelstra_core::msg::Coded::new(
            "store.projects_unavailable",
            format!("projects unavailable: {detail}"),
            &[("detail", detail)],
        ));
        Vec::new()
    })
}

/// Serializes read-modify-write of the projects file: two UI calls adding projects at once must not drop one.
pub(crate) static STORE_LOCK: Mutex<()> = Mutex::new(());

/// The list to modify and save back. Unlike `load`, a file that exists but cannot be read is an error, so an
/// empty list is never saved over projects we merely failed to read (a corrupt file was moved to `.bad` by then).
pub(crate) fn load_for_update(store: &Path) -> Result<Vec<Project>> {
    try_load(store)
}

pub(crate) fn save(store: &Path, v: &[Project]) -> Result<()> {
    // versioned envelope, fsynced temp file + rename, rotating backups: a crash never leaves a half-written list
    write_json(store, v)?;
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

/// Folders that must never become a project: a filesystem root, the user's home or a system directory
/// (a project is where agents work and where files are listed, written and deleted).
pub fn forbidden_root(p: &Path) -> bool {
    // `\?\` is dropped by `norm`; the same prefix with forward slashes (a path written that way) is dropped here
    let norm = |p: &Path| {
        let n = axelstra_core::paths::norm_path(p);
        n.strip_prefix("//?/").map(String::from).unwrap_or(n)
    };
    let n = norm(p);
    if p.parent().is_none() || n.is_empty() || (n.len() == 2 && n.ends_with(':')) {
        return true;
    }
    if ["USERPROFILE", "HOME"].iter().filter_map(std::env::var_os).any(|h| norm(Path::new(&h)) == n) {
        return true;
    }
    // a top-level system folder: /etc, C:/Windows, C:/Program Files...
    let unix_sys = ["etc", "usr", "bin", "sbin", "lib", "var", "boot", "dev", "proc", "sys", "system", "library", "private"];
    let win_sys = ["windows", "program files", "program files (x86)", "programdata", "users"];
    let top = if n.as_bytes().get(1) == Some(&b':') { n[2..].trim_start_matches('/') } else { n.trim_start_matches('/') };
    !top.contains('/') && (unix_sys.contains(&top) || win_sys.contains(&top))
}

/// The folder a project at `path` would have: its repository root, or the folder itself outside a repository.
/// Refuses filesystem roots, the home folder and system folders.
pub fn resolve_root(path: &str) -> Result<PathBuf> {
    // a folder outside any repository is accepted as it is: the project can start from prompts and `git init` later
    let root = match git::repo_root(Path::new(path)) {
        Ok(r) => r,
        Err(_) if Path::new(path).is_dir() => git::canon(Path::new(path))?,
        Err(e) => return Err(coded!("project.not_folder", "{path} is not a folder: {detail}", path = path, detail = format!("{e:#}"))),
    };
    if forbidden_root(&root) {
        return Err(coded!(
            "project.forbidden_root",
            "{path} cannot be a project: it is a filesystem root, the home folder or a system folder",
            path = root.display()
        ));
    }
    Ok(root)
}

pub fn add(store: &Path, path: &str) -> Result<Project> {
    let root = resolve_root(path)?;
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
    let p = Project { id, name, path: root_s, setup: None, parents: BTreeMap::new() };
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
    let p = v.iter_mut().find(|p| p.id == id).ok_or_else(|| coded!("project.unknown", "unknown project {id}", id = id))?;
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
    for p in try_load(store)? {
        let Ok(list) = git::worktree_list(Path::new(&p.path)) else { continue };
        for w in list {
            if axelstra_core::paths::same(&w.path, path) {
                return Ok(PathBuf::from(w.path));
            }
        }
    }
    Err(coded!("worktree.unknown", "not a worktree of a registered project: {path}", path = path))
}

pub fn find(store: &Path, id: &str) -> Result<Project> {
    try_load(store)?.into_iter().find(|p| p.id == id).ok_or_else(|| coded!("project.unknown", "unknown project {id}", id = id))
}

/// New worktree on a new branch `ax/<name>` from the repo's current HEAD.
/// Lives next to the repo in `.axelstra-wt/` so paths stay short.
pub fn worktree_create(p: &Project, name: &str) -> Result<git::Worktree> {
    Ok(create_at(p, name, "HEAD", true)?.0)
}

/// Like `worktree_create`, plus what `.worktreeinclude` copied into the new worktree (and what it left out).
pub fn worktree_create_report(p: &Project, name: &str) -> Result<(git::Worktree, crate::wtenv::Report)> {
    create_at(p, name, "HEAD", true)
}

/// Worktree on branch `ax/pr-<n>` holding the head of GitHub pull request `<n>`.
pub fn worktree_from_pr(p: &Project, number: u64) -> Result<git::Worktree> {
    Ok(worktree_from_pr_report(p, number)?.0)
}

/// Same, plus a warning when the project's `.worktreeinclude` would have copied git-ignored files (`.env`, keys):
/// the code of a pull request is not trusted, so nothing is copied into its worktree and the user is told.
pub fn worktree_from_pr_report(p: &Project, number: u64) -> Result<(git::Worktree, Option<axelstra_core::msg::Coded>)> {
    let repo = Path::new(&p.path);
    git::must(repo, &["fetch", "origin", &format!("pull/{number}/head")]).map_err(|e| {
        coded!(
            "worktree.fetch_pr_failed",
            "could not fetch pull request {number} from origin: {detail}",
            number = number,
            detail = format!("{e:#}")
        )
    })?;
    let wt = create_at(p, &format!("pr-{number}"), "FETCH_HEAD", false)?.0;
    // its code is not the user's: no agent started in it is told to trust the folder, even if the branch is renamed
    git::mark_untrusted(Path::new(&wt.path))?;
    let warning = crate::wtenv::plan(repo).ok().filter(|pl| !pl.files.is_empty()).map(|pl| {
        axelstra_core::msg::Coded::new(
            "worktree.pr_env_not_copied",
            format!(
                "{} ignored file(s) from .worktreeinclude ({}) were NOT copied into this pull request's worktree, because its code is not trusted; copy them by hand if you need them",
                pl.files.len(),
                pl.files.join(", ")
            ),
            &[("count", pl.files.len().to_string()), ("files", pl.files.join(", "))],
        )
    });
    Ok((wt, warning))
}

/// Folder of the worktree `name` of project `p`.
fn wt_dir(p: &Project, name: &str) -> Result<PathBuf> {
    let repo = Path::new(&p.path);
    Ok(repo.parent().ok_or_else(|| coded!("worktree.no_parent_dir", "repo has no parent directory"))?.join(".axelstra-wt").join(format!(
        "{}-{}",
        slug(&p.name),
        name
    )))
}

/// `copy_env`: also copy the git-ignored files `.worktreeinclude` selects (never for code that is not the user's own).
pub(crate) fn create_at(p: &Project, name: &str, base: &str, copy_env: bool) -> Result<(git::Worktree, crate::wtenv::Report)> {
    let n = slug(name);
    if n.is_empty() {
        return Err(coded!("worktree.name_empty", "worktree name is empty"));
    }
    let repo = Path::new(&p.path);
    if !git::is_repo(repo) {
        return Err(coded!(
            "worktree.not_repo",
            "this project is not a git repository yet: run `git init` in the folder so you can create worktrees"
        ));
    }
    let dir = wt_dir(p, &n)?;
    if dir.exists() {
        return Err(coded!("worktree.dir_exists", "{path} already exists", path = dir.display()));
    }
    std::fs::create_dir_all(dir.parent().unwrap())?;
    git::worktree_add(repo, &dir, &format!("ax/{n}"), base)?;
    // untracked config (.env...) the project asked for: a failure here never undoes the worktree
    let report = if copy_env { crate::wtenv::copy_into(repo, &dir) } else { crate::wtenv::Report::default() };
    let wt = git::worktree_list(repo)?
        .into_iter()
        .find(|w| Path::new(&w.path).file_name() == dir.file_name())
        .ok_or_else(|| coded!("worktree.not_listed", "worktree created but not listed"))?;
    Ok((wt, report))
}

/// Renames a worktree: its folder and its branch move together to `ax/<name>`. Not for the main worktree, not while
/// an agent or terminal in it is busy (`busy`, known to the caller), and never over something that exists.
/// Returns the renamed worktree.
pub fn worktree_rename(p: &Project, path: &str, name: &str, busy: bool) -> Result<git::Worktree> {
    let repo = Path::new(&p.path);
    if !axelstra_core::card::valid_slug(name) {
        return Err(coded!("worktree.bad_name", "invalid name: use only letters, digits, '.', '_' and '-' (not starting with a dot)"));
    }
    let wt = git::worktree_list(repo)?
        .into_iter()
        .find(|w| axelstra_core::paths::same(&w.path, path))
        .ok_or_else(|| coded!("worktree.not_of_project", "not a worktree of this project"))?;
    if wt.is_main {
        return Err(coded!("worktree.main_rename", "cannot rename the main worktree"));
    }
    if busy {
        return Err(coded!("worktree.busy", "there is an active session in this worktree: wait until it is idle before renaming"));
    }
    let old_branch = wt.branch.clone().ok_or_else(|| coded!("worktree.detached", "the worktree has no branch (detached HEAD)"))?;
    let new_branch = format!("ax/{name}");
    let to = wt_dir(p, name)?;
    if to.exists() {
        return Err(coded!("worktree.dir_exists", "{path} already exists", path = to.display()));
    }
    if git::run(repo, &["rev-parse", "--verify", "--quiet", &format!("refs/heads/{new_branch}")])?.ok {
        return Err(coded!("worktree.branch_exists", "branch {branch} already exists", branch = new_branch));
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
    git::worktree_list(repo)?
        .into_iter()
        .find(|w| w.branch.as_deref() == Some(new_branch.as_str()))
        .ok_or_else(|| coded!("worktree.not_listed", "renamed worktree not listed"))
}

pub fn worktree_remove(p: &Project, path: &str) -> Result<()> {
    let repo = Path::new(&p.path);
    let wt = git::worktree_list(repo)?
        .into_iter()
        .find(|w| axelstra_core::paths::same(&w.path, path))
        .ok_or_else(|| coded!("worktree.not_of_project", "not a worktree of this project"))?;
    if wt.is_main {
        return Err(coded!("worktree.main_remove", "cannot remove the main worktree"));
    }
    git::worktree_remove(repo, Path::new(&wt.path))?;
    if let Some(b) = wt.branch {
        git::branch_delete(repo, &b)?;
    }
    Ok(())
}

/// The non-main worktree `path` of project `p`, as git lists it.
fn plain_worktree(p: &Project, path: &str) -> Result<git::Worktree> {
    let want = norm_path(path);
    let wt = git::worktree_list(Path::new(&p.path))?
        .into_iter()
        .find(|w| norm_path(&w.path) == want)
        .ok_or_else(|| coded!("worktree.not_of_project", "not a worktree of this project"))?;
    if wt.is_main {
        return Err(coded!("worktree.main_in_group", "the main worktree cannot be part of a group"));
    }
    Ok(wt)
}

/// True when following `parents` upwards from `start` reaches `target`.
fn reaches(parents: &BTreeMap<String, String>, start: &str, target: &str) -> bool {
    let mut cur = start.to_string();
    // the chain cannot be longer than the map: a loop already in the file (hand edited) still ends
    for _ in 0..=parents.len() {
        if cur == target {
            return true;
        }
        match parents.get(&cur) {
            Some(next) => cur = next.clone(),
            None => return false,
        }
    }
    true
}

/// Checks that `parent` is a worktree of `p` that `path` may move under: same project, not the main worktree, not
/// itself, and not one of its own descendants (that would make a cycle). With `path` None (a worktree not created
/// yet) only the parent is checked.
pub fn check_parent(p: &Project, path: Option<&str>, parent: &str) -> Result<()> {
    // the main worktree keeps its own message; any other refusal means the group is not one of this project's worktrees
    let par = plain_worktree(p, parent).map_err(|e| {
        if axelstra_core::msg::find(&e).is_some_and(|c| c.code == "worktree.main_in_group") {
            e
        } else {
            coded!("worktree.group_not_worktree", "the chosen group is not a worktree of this project")
        }
    })?;
    let Some(path) = path else { return Ok(()) };
    let me = plain_worktree(p, path)?;
    let (me, par) = (norm_path(&me.path), norm_path(&par.path));
    if me == par {
        return Err(coded!("worktree.group_self", "a worktree cannot be placed inside itself"));
    }
    if reaches(&p.parents, &par, &me) {
        return Err(coded!("worktree.group_cycle", "that would create a cycle: the chosen group is already inside this worktree"));
    }
    Ok(())
}

/// Puts the worktree `path` of project `id` under `parent`, or back at the top level when `parent` is None.
/// Replaces whatever the worktree had before (also for a new worktree that reuses an old path).
pub fn set_parent(store: &Path, id: &str, path: &str, parent: Option<&str>) -> Result<()> {
    let _g = STORE_LOCK.lock_recover();
    let mut v = load_for_update(store)?;
    let p = v.iter_mut().find(|p| p.id == id).ok_or_else(|| coded!("project.unknown", "unknown project {id}", id = id))?;
    let me = norm_path(&plain_worktree(p, path)?.path);
    match parent.map(str::trim).filter(|s| !s.is_empty()) {
        Some(par) => {
            check_parent(p, Some(path), par)?;
            let par = norm_path(&plain_worktree(p, par)?.path);
            p.parents.insert(me, par);
        }
        None => {
            p.parents.remove(&me);
        }
    }
    save(store, &v)
}

/// Forgets the parent of a removed worktree, and the link of its children.
pub fn forget_parent(store: &Path, id: &str, path: &str) -> Result<()> {
    let _g = STORE_LOCK.lock_recover();
    let mut v = load_for_update(store)?;
    let Some(p) = v.iter_mut().find(|p| p.id == id) else { return Ok(()) };
    let gone = norm_path(path);
    let before = p.parents.len();
    p.parents.retain(|k, par| *k != gone && *par != gone);
    if p.parents.len() != before {
        save(store, &v)?;
    }
    Ok(())
}

/// A renamed worktree keeps its place: its own link and the links of its children follow the new path.
pub fn rename_parent(store: &Path, id: &str, old: &str, new: &str) -> Result<()> {
    let _g = STORE_LOCK.lock_recover();
    let mut v = load_for_update(store)?;
    let Some(p) = v.iter_mut().find(|p| p.id == id) else { return Ok(()) };
    let (old, new) = (norm_path(old), norm_path(new));
    let moved: BTreeMap<String, String> = p
        .parents
        .iter()
        .map(|(k, par)| (if *k == old { new.clone() } else { k.clone() }, if *par == old { new.clone() } else { par.clone() }))
        .collect();
    if moved != p.parents {
        p.parents = moved;
        save(store, &v)?;
    }
    Ok(())
}

/// The parent of each worktree as the UI wants it: worktree path -> parent path, both as git lists them.
/// Pairs whose worktrees are gone are left out.
pub fn parent_paths(p: &Project, worktrees: &[git::Worktree]) -> BTreeMap<String, String> {
    let real: BTreeMap<String, &str> = worktrees.iter().filter(|w| !w.is_main).map(|w| (norm_path(&w.path), w.path.as_str())).collect();
    worktrees
        .iter()
        .filter(|w| !w.is_main)
        .filter_map(|w| {
            let par = p.parents.get(&norm_path(&w.path))?;
            real.get(par).map(|r| (w.path.clone(), r.to_string()))
        })
        .collect()
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
            Project { id: "gone".into(), name: "gone".into(), path: gone, setup: None, parents: BTreeMap::new() },
            Project { id: "real".into(), name: "real".into(), path: real_s.clone(), setup: None, parents: BTreeMap::new() },
        ];
        save(&store, &list).unwrap();
        let wt = known_worktree(&store, &real_s).unwrap();
        let norm = axelstra_core::paths::norm;
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
        let _ = crate::store::take_notices();
        assert!(load(&store).is_empty());
        add(&store, &dir.to_string_lossy()).unwrap();
        let bad = std::fs::read_dir(&r)
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|p| p.file_name().unwrap().to_string_lossy().starts_with("projects.json.bad."))
            .expect("corrupt file kept as .bad.<ts>");
        assert_eq!(std::fs::read_to_string(bad).unwrap(), "[{ not json");
        assert!(crate::store::take_notices().iter().any(|n| n.contains("projects.json")));
        assert_eq!(load(&store).len(), 1);
        let _ = std::fs::remove_dir_all(&r);
    }

    #[test]
    fn an_unreadable_projects_file_is_never_overwritten_by_an_update() {
        let r = root("unreadable");
        let store = r.join("projects.json");
        // a folder where the file should be: it exists but cannot be read
        std::fs::create_dir_all(&store).unwrap();
        let _ = crate::store::take_notices();
        assert!(try_load(&store).is_err());
        assert!(known_worktree(&store, "x").is_err());
        assert!(find(&store, "x").is_err());
        // the read-only wrapper degrades to empty but surfaces a notice
        assert!(load(&store).is_empty());
        assert!(crate::store::take_notices().iter().any(|n| n.contains("projects unavailable")));
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
        (root, Project { id: "p".into(), name: "proj".into(), path, setup: None, parents: BTreeMap::new() })
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
        assert!(err(worktree_rename(&p, &a.path, "../x", false)).contains("invalid name"));
        assert!(err(worktree_rename(&p, &a.path, ".hidden", false)).contains("invalid name"));
        assert!(err(worktree_rename(&p, &a.path, "", false)).contains("invalid name"));
        assert_eq!(axelstra_core::msg::find(&worktree_rename(&p, &a.path, "busy", true).unwrap_err()).unwrap().code, "worktree.busy");
        assert!(err(worktree_rename(&p, &p.path, "main2", false)).contains("main worktree"));
        assert!(err(worktree_rename(&p, "C:/nowhere", "z", false)).contains("not a worktree"));
        // a branch of that name without a folder blocks it too
        git::must(Path::new(&p.path), &["branch", "ax/taken"]).unwrap();
        assert_eq!(
            axelstra_core::msg::find(&worktree_rename(&p, &b.path, "taken", false).unwrap_err()).unwrap().code,
            "worktree.branch_exists"
        );
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

    #[test]
    fn a_pull_request_worktree_gets_no_ignored_files_and_a_warning() {
        let (root, p) = setup("pr-env");
        let repo = Path::new(&p.path);
        std::fs::write(
            repo.join(".gitignore"),
            ".env
",
        )
        .unwrap();
        std::fs::write(repo.join(".env"), "SECRET=1").unwrap();
        std::fs::write(
            repo.join(crate::wtenv::INCLUDE_FILE),
            ".env
",
        )
        .unwrap();
        git::must(repo, &["add", ".gitignore", ".worktreeinclude"]).unwrap();
        git::must(repo, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qm", "env"]).unwrap();
        // the repository is its own "origin" and has a pull request ref
        git::must(repo, &["update-ref", "refs/pull/7/head", "HEAD"]).unwrap();
        git::must(repo, &["remote", "add", "origin", &p.path]).unwrap();
        let (w, warning) = worktree_from_pr_report(&p, 7).unwrap();
        assert!(!Path::new(&w.path).join(".env").exists(), "untrusted code must not see the .env");
        let warning = warning.expect("the user is told");
        assert_eq!(warning.code, "worktree.pr_env_not_copied");
        assert!(warning.text.contains(".env") && warning.text.contains("NOT copied"), "{warning}");
        // M8c: a pull request's worktree is never trusted for an unattended agent, even if its branch is renamed
        let wd = Path::new(&w.path);
        assert!(git::is_pr_code(wd) && !git::folder_trusted(wd));
        git::must(wd, &["branch", "-m", "ax/renamed"]).unwrap();
        assert!(git::is_pr_code(wd) && !git::folder_trusted(wd), "the marker outlives a rename");
        let own = create_at(&p, "mine", "HEAD", false).unwrap().0;
        assert!(git::folder_trusted(Path::new(&own.path)), "our own ax/* worktree is trusted");
        assert!(!git::folder_trusted(Path::new(&p.path)), "the main checkout is not on an ax/* branch");
        let _ = std::fs::remove_dir_all(&root);
    }
}

#[cfg(test)]
mod parent_tests {
    use super::*;

    fn setup(tag: &str) -> (PathBuf, Project, PathBuf) {
        let root = std::env::temp_dir().join(format!("axl-parent-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let repo = root.join("proj");
        std::fs::create_dir_all(&repo).unwrap();
        git::must(&repo, &["init", "-q", "-b", "main"]).unwrap();
        std::fs::write(repo.join("a.txt"), "1").unwrap();
        git::must(&repo, &["add", "."]).unwrap();
        git::must(&repo, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qm", "init"]).unwrap();
        let path = git::canon(&repo).unwrap().to_string_lossy().to_string();
        let p = Project { id: "p".into(), name: "proj".into(), path, setup: None, parents: BTreeMap::new() };
        let store = root.join("projects.json");
        save(&store, std::slice::from_ref(&p)).unwrap();
        (root, p, store)
    }

    fn reload(store: &Path) -> Project {
        load(store).remove(0)
    }

    #[test]
    fn parent_is_saved_survives_a_reload_and_can_be_cleared() {
        let (root, p, store) = setup("ok");
        let a = worktree_create(&p, "a").unwrap();
        let b = worktree_create(&p, "b").unwrap();
        set_parent(&store, "p", &b.path, Some(&a.path)).unwrap();
        let again = reload(&store);
        assert_eq!(again.parents.get(&norm_path(&b.path)), Some(&norm_path(&a.path)));
        let all = git::worktree_list(Path::new(&p.path)).unwrap();
        assert_eq!(parent_paths(&again, &all).get(&b.path), Some(&a.path));
        set_parent(&store, "p", &b.path, None).unwrap();
        assert!(reload(&store).parents.is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn parent_refuses_self_cycles_main_and_strangers() {
        let (root, p, store) = setup("no");
        let a = worktree_create(&p, "a").unwrap();
        let b = worktree_create(&p, "b").unwrap();
        let c = worktree_create(&p, "c").unwrap();
        let err = |r: Result<()>| format!("{:#}", r.unwrap_err());
        assert!(err(set_parent(&store, "p", &a.path, Some(&a.path))).contains("inside itself"));
        set_parent(&store, "p", &b.path, Some(&a.path)).unwrap();
        set_parent(&store, "p", &c.path, Some(&b.path)).unwrap();
        // a under c would close the loop a -> c -> b -> a
        assert!(err(set_parent(&store, "p", &a.path, Some(&c.path))).contains("cycle"));
        assert!(err(set_parent(&store, "p", &a.path, Some(&p.path))).contains("main worktree"));
        assert!(err(set_parent(&store, "p", &p.path, Some(&a.path))).contains("main worktree"));
        assert!(err(set_parent(&store, "p", &a.path, Some("C:/nowhere"))).contains("worktree of this project"));
        assert!(err(set_parent(&store, "other", &a.path, Some(&b.path))).contains("unknown project"));
        // the refusals changed nothing
        assert_eq!(reload(&store).parents.len(), 2);
        // a worktree not created yet: only the parent is checked
        assert!(check_parent(&p, None, &a.path).is_ok());
        assert!(check_parent(&p, None, "C:/nowhere").is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn forgetting_a_worktree_drops_it_and_frees_its_children() {
        let (root, p, store) = setup("forget");
        let a = worktree_create(&p, "a").unwrap();
        let b = worktree_create(&p, "b").unwrap();
        set_parent(&store, "p", &b.path, Some(&a.path)).unwrap();
        forget_parent(&store, "p", &a.path).unwrap();
        assert!(reload(&store).parents.is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn renaming_moves_the_links() {
        let (root, p, store) = setup("rename");
        let a = worktree_create(&p, "a").unwrap();
        let b = worktree_create(&p, "b").unwrap();
        set_parent(&store, "p", &b.path, Some(&a.path)).unwrap();
        rename_parent(&store, "p", &a.path, "C:/x/proj-z").unwrap();
        assert_eq!(reload(&store).parents.get(&norm_path(&b.path)), Some(&norm_path("C:/x/proj-z")));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_loop_in_the_file_does_not_hang() {
        let mut m = BTreeMap::new();
        m.insert("a".to_string(), "b".to_string());
        m.insert("b".to_string(), "a".to_string());
        assert!(reaches(&m, "a", "zzz"));
        assert!(!reaches(&BTreeMap::new(), "a", "b"));
    }
}

#[cfg(test)]
mod root_tests {
    use super::forbidden_root;
    use std::path::Path;

    #[test]
    fn roots_and_system_folders_are_refused() {
        // drive-letter paths are only paths on Windows; elsewhere they are just odd relative names
        let win_bad = ["C:/", "C:/Windows", "c:/program files", "D:/Users", "//?/C:/Windows"];
        for bad in ["/", "/etc", "/usr/"].into_iter().chain(win_bad.into_iter().filter(|_| cfg!(windows))) {
            assert!(forbidden_root(Path::new(bad)), "{bad}");
        }
        let win_ok = ["C:/Users/a/proj", "D:/dev/app"];
        for ok in ["/home/a/proj", "/usr/local/src/proj"].into_iter().chain(win_ok.into_iter().filter(|_| cfg!(windows))) {
            assert!(!forbidden_root(Path::new(ok)), "{ok}");
        }
    }
}
