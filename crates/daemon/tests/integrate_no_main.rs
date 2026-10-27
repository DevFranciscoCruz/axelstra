//! `integrate_with` when the default branch is checked out nowhere (the main checkout sits on another branch).
mod common;
use axelstra_core::git;
use axelstra_daemon::integrate::{self, Outcome};
use axelstra_daemon::projects::{self, Project};
use std::path::{Path, PathBuf};

struct Fx {
    root: PathBuf,
    p: Project,
}

impl Drop for Fx {
    fn drop(&mut self) {
        git::purge_trash(&self.root);
        let _ = git::remove_tree(&self.root);
    }
}

fn g(dir: &Path, args: &[&str]) -> String {
    let mut a = vec!["-c", "user.name=t", "-c", "user.email=t@t", "-c", "core.autocrlf=false"];
    a.extend_from_slice(args);
    let o = git::run(dir, &a).unwrap();
    assert!(o.ok, "git {args:?}: {}", o.stderr);
    o.stdout.trim().to_string()
}

fn work(dir: &Path, file: &str, text: &str) {
    std::fs::write(dir.join(file), text).unwrap();
    g(dir, &["add", file]);
    g(dir, &["commit", "-q", "-m", file]);
}

/// Project whose main checkout is on `side`; `main` is checked out nowhere. Returns the fixture and the lead worktree.
fn fx(tag: &str) -> (Fx, git::Worktree) {
    common::data_dir();
    let root = std::env::temp_dir().join(format!("axl-intnm-{tag}-{}", std::process::id()));
    let _ = git::remove_tree(&root);
    let repo = root.join("proj");
    std::fs::create_dir_all(&repo).unwrap();
    g(&repo, &["init", "-q", "-b", "main"]);
    work(&repo, "base.txt", "base\n");
    let store = root.join("projects.json");
    let p = projects::add(&store, &repo.to_string_lossy()).unwrap();
    let w = projects::worktree_create(&p, "x").unwrap();
    g(&repo, &["checkout", "-q", "-b", "side"]);
    work(&repo, "side.txt", "side\n");
    let wt = git::worktree_list(&repo).unwrap().into_iter().find(|x| axelstra_core::paths::same(&x.path, &w.path)).unwrap();
    (Fx { root, p }, wt)
}

fn listed(repo: &Path) -> usize {
    g(repo, &["worktree", "list", "--porcelain"]).lines().filter(|l| l.starts_with("worktree ")).count()
}

fn code(e: anyhow::Error) -> String {
    axelstra_core::msg::find(&e).map(|c| c.code.clone()).unwrap_or_else(|| format!("{e:#}"))
}

#[test]
fn fast_forward_moves_only_the_ref() {
    let (f, wt) = fx("ff");
    let repo = Path::new(&f.p.path);
    work(Path::new(&wt.path), "a.txt", "a\n");
    let tip = g(Path::new(&wt.path), &["rev-parse", "HEAD"]);
    let (head, side_files) = (git::head(repo).unwrap(), std::fs::read_dir(repo).unwrap().count());
    let out = integrate::integrate(&f.p, &wt).unwrap();
    assert!(matches!(out, Outcome::Integrated { how: "ff", removed: true, .. }), "{out:?}");
    assert_eq!(g(repo, &["rev-parse", "refs/heads/main"]), tip);
    assert_eq!(git::head(repo).unwrap(), head, "checkout HEAD untouched");
    assert_eq!(git::current_branch(repo).unwrap(), "side");
    assert_eq!(std::fs::read_dir(repo).unwrap().count(), side_files);
    assert!(!repo.join("a.txt").exists());
    assert_eq!(listed(repo), 1);
}

#[test]
fn diverged_branch_merges_in_a_throwaway_worktree() {
    let (f, wt) = fx("merge");
    let repo = Path::new(&f.p.path);
    work(Path::new(&wt.path), "a.txt", "a\n");
    // main gets its own commit, without checking it out
    let tmp = f.root.join("other");
    g(repo, &["worktree", "add", "-q", &tmp.to_string_lossy(), "main"]);
    work(&tmp, "m.txt", "m\n");
    g(repo, &["worktree", "remove", "--force", &tmp.to_string_lossy()]);
    let out = integrate::integrate(&f.p, &wt).unwrap();
    assert!(matches!(out, Outcome::Integrated { how: "merge", removed: true, .. }), "{out:?}");
    let parents = g(repo, &["rev-list", "--parents", "-n", "1", "refs/heads/main"]);
    assert_eq!(parents.split_whitespace().count(), 3, "merge commit: {parents}");
    assert_eq!(g(repo, &["show", "refs/heads/main:a.txt"]).trim(), "a");
    assert_eq!(g(repo, &["show", "refs/heads/main:m.txt"]).trim(), "m");
    assert_eq!(git::current_branch(repo).unwrap(), "side");
    assert_eq!(listed(repo), 1, "no throwaway worktree left");
}

#[test]
fn conflict_changes_nothing() {
    let (f, wt) = fx("conflict");
    let repo = Path::new(&f.p.path);
    work(Path::new(&wt.path), "base.txt", "theirs\n");
    let tmp = f.root.join("other");
    g(repo, &["worktree", "add", "-q", &tmp.to_string_lossy(), "main"]);
    work(&tmp, "base.txt", "ours\n");
    g(repo, &["worktree", "remove", "--force", &tmp.to_string_lossy()]);
    let before = g(repo, &["rev-parse", "refs/heads/main"]);
    assert_eq!(code(integrate::integrate(&f.p, &wt).unwrap_err()), "integrate.conflict");
    assert_eq!(g(repo, &["rev-parse", "refs/heads/main"]), before);
    assert!(Path::new(&wt.path).exists());
    assert_eq!(listed(repo), 2, "main checkout + the agent worktree only");
}

#[test]
fn a_moved_ref_fails_the_compare_and_swap() {
    let (f, wt) = fx("cas");
    let repo = Path::new(&f.p.path);
    work(
        Path::new(&wt.path),
        "a.txt",
        "a
",
    );
    let tip = g(Path::new(&wt.path), &["rev-parse", "HEAD"]);
    let stale = g(repo, &["rev-parse", "refs/heads/main"]);
    g(repo, &["update-ref", "refs/heads/main", "side"]);
    let e = integrate::cas_ref(repo, "main", "ax/x", &tip, &stale).unwrap_err();
    assert_eq!(code(e), "integrate.default_moved");
    assert_eq!(g(repo, &["rev-parse", "refs/heads/main"]), g(repo, &["rev-parse", "side"]), "ref left as the other writer set it");
    // up to date old value works
    integrate::cas_ref(repo, "main", "ax/x", &tip, &g(repo, &["rev-parse", "side"])).unwrap();
}
