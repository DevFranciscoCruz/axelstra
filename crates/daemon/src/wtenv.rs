//! Untracked config for new worktrees. A fresh worktree has no `.env` or other git-ignored files, so agents fail
//! on it. `.worktreeinclude` in the project root lists gitignore-style patterns of IGNORED files to copy from the
//! main checkout into every new worktree. Only files that git itself ignores are considered, they must really live
//! inside the main checkout (no `..`, no symlinks out, never `.git`), and the amount is capped. Whatever is left
//! out is reported, never dropped silently.

use anyhow::Result;
use axelstra_core::git;
use ignore::gitignore::GitignoreBuilder;
use serde::Serialize;
use std::path::{Component, Path};

pub const INCLUDE_FILE: &str = ".worktreeinclude";
pub const MAX_FILE_BYTES: u64 = 5 * 1024 * 1024;
pub const MAX_FILES: usize = 50;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Skip {
    pub path: String,
    pub reason: String,
}

#[derive(Debug, Default, Serialize)]
pub struct Plan {
    /// the project has a `.worktreeinclude` at all
    pub has_file: bool,
    /// paths relative to the project root, forward slashes
    pub files: Vec<String>,
    pub skipped: Vec<Skip>,
}

#[derive(Debug, Default, Serialize)]
pub struct Report {
    pub copied: Vec<String>,
    pub skipped: Vec<Skip>,
}

fn skip(path: &str, reason: &str) -> Skip {
    Skip { path: path.to_string(), reason: reason.to_string() }
}

/// A relative path made only of plain names, none of them `.git`.
fn safe_rel(rel: &str) -> bool {
    let p = Path::new(rel);
    !rel.is_empty()
        && p.is_relative()
        && p.components().all(|c| matches!(c, Component::Normal(n) if !n.to_string_lossy().eq_ignore_ascii_case(".git")))
}

/// Patterns that could point outside the checkout are left out (and reported); comments and blanks go too.
fn clean_patterns(text: &str, skipped: &mut Vec<Skip>) -> Vec<String> {
    let mut out = Vec::new();
    for l in text.lines() {
        let l = l.trim();
        if l.is_empty() || l.starts_with('#') {
            continue;
        }
        let body = l.trim_start_matches('!');
        let bad = body.split(['/', '\\']).any(|s| s == "..") || body.contains(':') || body.starts_with('\\');
        if bad {
            skipped.push(skip(l, "padrão fora do projeto"));
        } else {
            out.push(l.to_string());
        }
    }
    out
}

/// What `.worktreeinclude` would copy from `repo` (the main checkout) right now.
pub fn plan(repo: &Path) -> Result<Plan> {
    let mut plan = Plan::default();
    let Ok(text) = std::fs::read_to_string(repo.join(INCLUDE_FILE)) else { return Ok(plan) };
    plan.has_file = true;
    if !git::is_repo(repo) {
        return Ok(plan);
    }
    let patterns = clean_patterns(&text, &mut plan.skipped);
    if patterns.is_empty() {
        return Ok(plan);
    }
    let mut b = GitignoreBuilder::new(repo);
    for p in &patterns {
        b.add_line(None, p)?;
    }
    let want = b.build()?;
    let root = std::fs::canonicalize(repo)?;
    // untracked AND ignored: tracked files come with the checkout, and an unignored file is not config to hide
    let listed = git::must(repo, &["-c", "core.quotepath=false", "ls-files", "-z", "--others", "--ignored", "--exclude-standard"])?;
    let mut names: Vec<&str> = listed.split('\0').filter(|s| !s.is_empty()).collect();
    names.sort_unstable();
    for rel in names {
        if !want.matched_path_or_any_parents(rel, false).is_ignore() {
            continue;
        }
        if !safe_rel(rel) {
            plan.skipped.push(skip(rel, "caminho não permitido"));
            continue;
        }
        let full = repo.join(rel);
        let Ok(md) = std::fs::symlink_metadata(&full) else { continue };
        if md.file_type().is_symlink() {
            plan.skipped.push(skip(rel, "ligação simbólica"));
            continue;
        }
        if !md.is_file() {
            continue;
        }
        // a symlinked folder in the middle of the path would lead out of the checkout
        if !std::fs::canonicalize(&full).map(|c| c.starts_with(&root)).unwrap_or(false) {
            plan.skipped.push(skip(rel, "fora do projeto"));
            continue;
        }
        if md.len() > MAX_FILE_BYTES {
            plan.skipped.push(skip(rel, "maior que 5 MB"));
            continue;
        }
        if plan.files.len() >= MAX_FILES {
            plan.skipped.push(skip(rel, "limite de 50 ficheiros"));
            continue;
        }
        plan.files.push(rel.to_string());
    }
    Ok(plan)
}

/// Copies what `.worktreeinclude` selects from `repo` into the new worktree `wt`. Never overwrites a file the
/// checkout already has. Problems are reported per file; the worktree itself is already good by now.
pub fn copy_into(repo: &Path, wt: &Path) -> Report {
    let mut report = Report::default();
    let plan = match plan(repo) {
        Ok(p) => p,
        Err(e) => {
            report.skipped.push(skip(INCLUDE_FILE, &format!("{e:#}")));
            return report;
        }
    };
    report.skipped = plan.skipped;
    let Ok(wt_root) = std::fs::canonicalize(wt) else { return report };
    for rel in plan.files {
        let to = wt.join(&rel);
        if to.symlink_metadata().is_ok() {
            report.skipped.push(skip(&rel, "já existe no worktree"));
            continue;
        }
        let done = (|| -> Result<bool> {
            let Some(dir) = to.parent() else { return Ok(false) };
            std::fs::create_dir_all(dir)?;
            // a tracked symlink folder in the worktree must not carry the copy outside it
            if !std::fs::canonicalize(dir)?.starts_with(&wt_root) {
                return Ok(false);
            }
            std::fs::copy(repo.join(&rel), &to)?;
            Ok(true)
        })();
        match done {
            Ok(true) => report.copied.push(rel),
            Ok(false) => report.skipped.push(skip(&rel, "fora do worktree")),
            Err(e) => report.skipped.push(skip(&rel, &format!("{e:#}"))),
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn repo(tag: &str) -> PathBuf {
        let r = std::env::temp_dir().join(format!("axl-wtenv-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&r);
        std::fs::create_dir_all(&r).unwrap();
        git::must(&r, &["init", "-q"]).unwrap();
        r
    }

    fn put(r: &Path, rel: &str, text: &str) {
        let p = r.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    #[test]
    fn copies_only_ignored_files_the_patterns_select() {
        let r = repo("sel");
        put(&r, ".gitignore", ".env\n.env.*\n!.env.example\nbuild.log\nconfig/local.json\n");
        put(&r, ".env", "A=1");
        put(&r, ".env.local", "B=2");
        put(&r, ".env.example", "tracked");
        put(&r, "build.log", "noise");
        put(&r, "config/local.json", "{}");
        put(&r, "src/main.rs", "fn main() {}");
        put(&r, INCLUDE_FILE, "# secrets\n.env*\nconfig/*.json\n");
        git::must(&r, &["add", ".gitignore", ".env.example", "src"]).unwrap();
        let p = plan(&r).unwrap();
        assert!(p.has_file);
        // .env.example is not ignored (negated, and tracked); build.log and src are not selected
        assert_eq!(p.files, vec![".env", ".env.local", "config/local.json"]);

        let wt = r.with_file_name(format!("{}-wt", r.file_name().unwrap().to_string_lossy()));
        let _ = std::fs::remove_dir_all(&wt);
        std::fs::create_dir_all(&wt).unwrap();
        put(&wt, ".env.local", "already here");
        let rep = copy_into(&r, &wt);
        assert_eq!(rep.copied, vec![".env", "config/local.json"]);
        assert_eq!(std::fs::read_to_string(wt.join(".env")).unwrap(), "A=1");
        assert_eq!(std::fs::read_to_string(wt.join(".env.local")).unwrap(), "already here");
        assert!(rep.skipped.iter().any(|s| s.path == ".env.local"));
        assert!(!wt.join("build.log").exists());
        let _ = std::fs::remove_dir_all(&wt);
        let _ = std::fs::remove_dir_all(&r);
    }

    #[test]
    fn no_include_file_copies_nothing() {
        let r = repo("none");
        put(&r, ".gitignore", ".env\n");
        put(&r, ".env", "A=1");
        let p = plan(&r).unwrap();
        assert!(!p.has_file && p.files.is_empty());
        let _ = std::fs::remove_dir_all(&r);
    }

    #[test]
    fn patterns_that_climb_out_are_refused() {
        let r = repo("esc");
        put(&r, ".gitignore", ".env\n");
        put(&r, ".env", "A=1");
        put(&r, "../axl-outside-secret.txt", "x");
        put(&r, INCLUDE_FILE, "../axl-outside-secret.txt\n!../x\nC:/Windows/win.ini\n\\evil\n.env\n");
        let p = plan(&r).unwrap();
        assert_eq!(p.files, vec![".env"]);
        assert_eq!(p.skipped.len(), 4, "{:?}", p.skipped);
        let wt = r.join("wt");
        std::fs::create_dir_all(&wt).unwrap();
        let rep = copy_into(&r, &wt);
        assert_eq!(rep.copied, vec![".env"]);
        let _ = std::fs::remove_file(r.join("../axl-outside-secret.txt"));
        let _ = std::fs::remove_dir_all(&r);
    }

    #[test]
    fn dot_git_and_odd_paths_are_never_safe() {
        assert!(safe_rel(".env"));
        assert!(safe_rel("a/b/c.json"));
        assert!(!safe_rel(".git/config"));
        assert!(!safe_rel("a/.GIT/hooks/x"));
        assert!(!safe_rel("../x"));
        assert!(!safe_rel("a/../../x"));
        assert!(!safe_rel("/abs"));
        assert!(!safe_rel(""));
    }

    #[test]
    fn symlinks_are_not_followed_out_of_the_checkout() {
        let r = repo("link");
        let outside = std::env::temp_dir().join(format!("axl-wtenv-secret-{}", std::process::id()));
        std::fs::write(&outside, "secret").unwrap();
        put(&r, ".gitignore", "leak\nlinkdir\n");
        put(&r, INCLUDE_FILE, "leak\nlinkdir/**\n");
        #[cfg(windows)]
        let made = std::os::windows::fs::symlink_file(&outside, r.join("leak")).is_ok();
        #[cfg(not(windows))]
        let made = std::os::unix::fs::symlink(&outside, r.join("leak")).is_ok();
        if made {
            let p = plan(&r).unwrap();
            assert!(p.files.is_empty(), "{:?}", p.files);
            assert!(p.skipped.iter().any(|s| s.path == "leak"));
        }
        let _ = std::fs::remove_file(&outside);
        let _ = std::fs::remove_dir_all(&r);
    }

    #[test]
    fn big_files_and_too_many_files_are_reported() {
        let r = repo("caps");
        put(&r, ".gitignore", "*.cfg\n");
        put(&r, INCLUDE_FILE, "*.cfg\n");
        std::fs::write(r.join("huge.cfg"), vec![0u8; (MAX_FILE_BYTES + 1) as usize]).unwrap();
        for i in 0..(MAX_FILES + 3) {
            put(&r, &format!("f{i:03}.cfg"), "x");
        }
        let p = plan(&r).unwrap();
        assert_eq!(p.files.len(), MAX_FILES);
        assert!(p.skipped.iter().any(|s| s.path == "huge.cfg" && s.reason.contains("5 MB")));
        assert_eq!(p.skipped.iter().filter(|s| s.reason.contains("limite")).count(), 3);
        let _ = std::fs::remove_dir_all(&r);
    }
}
