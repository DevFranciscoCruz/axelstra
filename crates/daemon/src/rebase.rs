//! `worktree.rebase`: rebase a worktree's branch onto the default branch in one step, only when nothing can be lost.
//! A conflict is never left for the user to untangle: the rebase is aborted and the branch is back where it was.

use anyhow::{bail, Context, Result};
use axelstra_core::git;
use serde_json::{json, Value};
use std::path::Path;

/// Rebases the worktree at `path` (a worktree of the project at `repo`) onto the project's default branch.
/// `busy` is true when an agent session works in it. Refused when the worktree is the main one, has no branch,
/// is busy, or has any change (untracked files included). Result:
/// `{ok:true, from, to}` or `{ok:false, conflicted:[files], restored:bool}` after a conflict.
pub fn rebase(repo: &Path, path: &str, busy: bool) -> Result<Value> {
    // the branch may have changed since the list was last cached
    git::forget_worktree_lists();
    let wt = git::worktree_list(repo)?
        .into_iter()
        .find(|w| axelstra_core::paths::same(&w.path, path))
        .context("not a worktree of this project")?;
    if wt.is_main {
        bail!("cannot rebase the main worktree");
    }
    let branch = wt.branch.clone().context("o worktree não tem ramo (detached HEAD)")?;
    if busy {
        bail!("há uma sessão ativa neste worktree: espera que fique inativa antes de fazer rebase");
    }
    let dir = Path::new(&wt.path);
    // a rebase or merge left half-way by hand is not ours to continue
    let gitdir = git::must(dir, &["rev-parse", "--path-format=absolute", "--git-dir"])?;
    if ["rebase-merge", "rebase-apply", "MERGE_HEAD"].iter().any(|n| Path::new(&gitdir).join(n).exists()) {
        bail!("há um rebase ou merge por acabar neste worktree");
    }
    let dirty = git::must(dir, &["status", "--porcelain", "--untracked-files=all"])?;
    if !dirty.is_empty() {
        bail!("o worktree tem alterações por guardar: faz commit ou stash antes do rebase");
    }
    let base = git::default_branch(repo);
    if branch == base {
        bail!("o worktree já está no ramo {base}");
    }
    let from = git::head(dir)?;
    let out = git::run(dir, &["rebase", &base])?;
    if out.ok {
        git::forget_worktree_lists();
        crate::overlaps::forget();
        return Ok(json!({"ok": true, "from": from, "to": git::head(dir)?, "base": base}));
    }
    // the unmerged paths exist only while the rebase is stopped; read them before aborting
    let conflicted: Vec<String> = git::run(dir, &["diff", "--name-only", "--diff-filter=U"])
        .map(|o| o.stdout.lines().map(|l| l.trim().replace('\\', "/")).filter(|l| !l.is_empty()).collect())
        .unwrap_or_default();
    let _ = git::run(dir, &["rebase", "--abort"]);
    let mut restored = git::head(dir).map(|h| h == from).unwrap_or(false);
    if !restored {
        // the tree was clean when we started, so putting the branch back by hand loses nothing
        let _ = git::run(dir, &["rebase", "--quit"]);
        let _ = git::run(dir, &["reset", "--hard", &from]);
        restored = git::head(dir).map(|h| h == from).unwrap_or(false);
    }
    git::forget_worktree_lists();
    if conflicted.is_empty() {
        bail!("o rebase falhou: {}", out.stderr.lines().last().unwrap_or("erro desconhecido"));
    }
    Ok(json!({"ok": false, "conflicted": conflicted, "restored": restored, "from": from, "base": base}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn g(cwd: &Path, args: &[&str]) {
        let o = Command::new("git")
            .current_dir(cwd)
            .args(["-c", "user.name=t", "-c", "user.email=t@t"])
            .args(args)
            .output()
            .unwrap();
        assert!(o.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&o.stderr));
    }

    fn setup(tag: &str) -> (std::path::PathBuf, String) {
        // the rebase runs git without `-c`: identity comes from the environment
        for (k, v) in [("GIT_AUTHOR_NAME", "t"), ("GIT_AUTHOR_EMAIL", "t@t"), ("GIT_COMMITTER_NAME", "t"), ("GIT_COMMITTER_EMAIL", "t@t")] {
            std::env::set_var(k, v);
        }
        let root = std::env::temp_dir().join(format!("axl-rebase-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let repo = root.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        g(&repo, &["init", "-q", "-b", "main"]);
        std::fs::write(repo.join("a.txt"), "1\n").unwrap();
        g(&repo, &["add", "."]);
        g(&repo, &["commit", "-qm", "init"]);
        let wt = root.join("wt");
        g(&repo, &["worktree", "add", "-q", "-b", "ax/topic", &wt.to_string_lossy(), "HEAD"]);
        (repo, wt.to_string_lossy().to_string())
    }

    #[test]
    fn clean_rebase_moves_the_branch_onto_main() {
        let (repo, wt) = setup("ok");
        std::fs::write(Path::new(&wt).join("b.txt"), "b").unwrap();
        g(Path::new(&wt), &["add", "."]);
        g(Path::new(&wt), &["commit", "-qm", "b"]);
        std::fs::write(repo.join("c.txt"), "c").unwrap();
        g(&repo, &["add", "."]);
        g(&repo, &["commit", "-qm", "c"]);
        let r = rebase(&repo, &wt, false).unwrap();
        assert_eq!(r["ok"], true, "{r}");
        assert_ne!(r["from"], r["to"]);
        assert!(Path::new(&wt).join("c.txt").exists() && Path::new(&wt).join("b.txt").exists());
    }

    #[test]
    fn dirty_busy_main_and_untracked_are_refused() {
        let (repo, wt) = setup("refuse");
        let w = Path::new(&wt);
        assert!(rebase(&repo, &wt, true).unwrap_err().to_string().contains("sessão ativa"));
        assert!(rebase(&repo, &repo.to_string_lossy(), false).unwrap_err().to_string().contains("main worktree"));
        std::fs::write(w.join("new.txt"), "x").unwrap();
        assert!(rebase(&repo, &wt, false).unwrap_err().to_string().contains("alterações"), "untracked file must refuse");
        std::fs::remove_file(w.join("new.txt")).unwrap();
        std::fs::write(w.join("a.txt"), "edited\n").unwrap();
        assert!(rebase(&repo, &wt, false).unwrap_err().to_string().contains("alterações"));
        g(w, &["checkout", "--", "a.txt"]);
        g(w, &["checkout", "-q", "--detach"]);
        assert!(rebase(&repo, &wt, false).unwrap_err().to_string().contains("detached"));
    }

    #[test]
    fn conflict_aborts_and_restores_the_original_head() {
        let (repo, wt) = setup("conflict");
        let w = Path::new(&wt);
        std::fs::write(w.join("a.txt"), "topic\n").unwrap();
        g(w, &["commit", "-qam", "topic"]);
        let before = git::head(w).unwrap();
        std::fs::write(repo.join("a.txt"), "main\n").unwrap();
        g(&repo, &["commit", "-qam", "main"]);
        let r = rebase(&repo, &wt, false).unwrap();
        assert_eq!(r["ok"], false, "{r}");
        assert_eq!(r["restored"], true);
        assert_eq!(r["conflicted"], json!(["a.txt"]));
        assert_eq!(git::head(w).unwrap(), before);
        assert!(git::must(w, &["status", "--porcelain"]).unwrap().is_empty());
        assert_eq!(std::fs::read_to_string(w.join("a.txt")).unwrap().trim(), "topic");
    }
}
