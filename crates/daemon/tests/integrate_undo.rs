//! `integrate::undo`: takes back an integration while nothing was built on it, and refuses (coded) when it cannot.
mod common;
use axelstra_core::git;
use axelstra_daemon::integrate::{self, Outcome, UndoArgs};
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

fn fx(tag: &str) -> Fx {
    common::data_dir();
    let root = std::env::temp_dir().join(format!("axl-undo-{tag}-{}", std::process::id()));
    let _ = git::remove_tree(&root);
    let repo = root.join("proj");
    std::fs::create_dir_all(&repo).unwrap();
    g(&repo, &["init", "-q", "-b", "main"]);
    work(&repo, "base.txt", "base\n");
    let p = projects::add(&root.join("projects.json"), &repo.to_string_lossy()).unwrap();
    Fx { root, p }
}

fn code(e: anyhow::Error) -> String {
    axelstra_core::msg::find(&e).map(|c| c.code.clone()).unwrap_or_else(|| format!("{e:#}"))
}

/// Integrates a worktree with one commit; returns the undo arguments and the old worktree path.
fn integrated(f: &Fx, name: &str) -> (UndoArgs, String) {
    let w = projects::worktree_create(&f.p, name).unwrap();
    work(Path::new(&w.path), &format!("{name}.txt"), "x\n");
    let wt = git::worktree_list(Path::new(&f.p.path)).unwrap().into_iter().find(|x| axelstra_core::paths::same(&x.path, &w.path)).unwrap();
    match integrate::integrate(&f.p, &wt).unwrap() {
        Outcome::Integrated { into, removed: true, prev_tip, new_tip, branch, branch_tip, .. } => {
            (UndoArgs { default_branch: into, prev_tip, new_tip, branch, branch_tip }, w.path)
        }
        o => panic!("{o:?}"),
    }
}

fn ref_of(repo: &Path, r: &str) -> Option<String> {
    git::run(repo, &["rev-parse", "--verify", "--quiet", r]).ok().filter(|o| o.ok).map(|o| o.stdout.trim().to_string())
}

#[test]
fn undo_puts_back_the_ref_the_checkout_the_branch_and_the_worktree() {
    let f = fx("ok");
    let repo = Path::new(&f.p.path);
    let before = ref_of(repo, "refs/heads/main").unwrap();
    let (a, old_path) = integrated(&f, "u1");
    assert_eq!(a.prev_tip, before);
    assert!(repo.join("u1.txt").exists());
    assert!(ref_of(repo, "refs/heads/ax/u1").is_none());

    let done = integrate::undo(&f.p, &a).unwrap();
    assert!(done.branch_restored);
    assert_eq!(ref_of(repo, "refs/heads/main").unwrap(), before);
    assert_eq!(ref_of(repo, "refs/heads/ax/u1").unwrap(), a.branch_tip, "no commit lost");
    assert!(!repo.join("u1.txt").exists(), "the checked-out default branch was reset with its ref");
    assert_eq!(g(repo, &["status", "--porcelain", "--untracked-files=no"]), "");
    let wt = done.worktree.expect("the worktree comes back");
    assert!(Path::new(&wt).join("u1.txt").exists());
    assert!(axelstra_core::paths::same(&wt, &old_path), "same folder as before: {wt} vs {old_path}");
}

#[test]
fn undo_with_the_default_branch_checked_out_nowhere_touches_no_checkout() {
    let f = fx("nowhere");
    let repo = Path::new(&f.p.path);
    g(repo, &["checkout", "-q", "-b", "side"]);
    let before = ref_of(repo, "refs/heads/main").unwrap();
    let (a, _) = integrated(&f, "u2");
    let head = git::head(repo).unwrap();
    integrate::undo(&f.p, &a).unwrap();
    assert_eq!(ref_of(repo, "refs/heads/main").unwrap(), before);
    assert_eq!(git::head(repo).unwrap(), head, "the main checkout stayed on side");
}

#[test]
fn undo_is_refused_when_the_default_branch_moved() {
    let f = fx("moved");
    let repo = Path::new(&f.p.path);
    let (a, _) = integrated(&f, "u3");
    work(repo, "later.txt", "later\n");
    let tip = ref_of(repo, "refs/heads/main").unwrap();
    assert_eq!(code(integrate::undo(&f.p, &a).unwrap_err()), "integrate.undo_moved");
    assert_eq!(ref_of(repo, "refs/heads/main").unwrap(), tip, "nothing changed");
    assert!(ref_of(repo, "refs/heads/ax/u3").is_none(), "no branch made by a refused undo");
}

#[test]
fn undo_is_refused_after_a_push_or_with_a_dirty_checkout() {
    let f = fx("push");
    let repo = Path::new(&f.p.path);
    let bare = f.root.join("origin.git");
    std::fs::create_dir_all(&bare).unwrap();
    g(&bare, &["init", "-q", "--bare", "-b", "main"]);
    g(repo, &["remote", "add", "origin", &bare.to_string_lossy()]);
    g(repo, &["push", "-q", "origin", "main"]);
    let (a, _) = integrated(&f, "u4");
    // dirty tracked file in the default branch's checkout
    std::fs::write(repo.join("base.txt"), "edited\n").unwrap();
    assert_eq!(code(integrate::undo(&f.p, &a).unwrap_err()), "integrate.default_dirty");
    g(repo, &["checkout", "-q", "--", "base.txt"]);
    g(repo, &["push", "-q", "origin", "main"]);
    assert_eq!(code(integrate::undo(&f.p, &a).unwrap_err()), "integrate.undo_pushed");
    assert_eq!(ref_of(repo, "refs/heads/main").unwrap(), a.new_tip);
}

#[test]
fn undo_refuses_arguments_that_are_not_what_integrate_reported() {
    let f = fx("bad");
    let (mut a, _) = integrated(&f, "u5");
    a.branch = "main".into();
    assert_eq!(code(integrate::undo(&f.p, &a).unwrap_err()), "integrate.undo_invalid");
    a.branch = "ax/u5".into();
    a.prev_tip = "--force".into();
    assert_eq!(code(integrate::undo(&f.p, &a).unwrap_err()), "integrate.undo_invalid");
}
