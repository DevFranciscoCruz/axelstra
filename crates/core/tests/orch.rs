//! End-to-end orchestration tests driven by tools/fake-agent.ps1.
#![cfg(windows)]

use axelstra_core::orch::{self, Options, Report};
use axelstra_core::{card, git};
use std::path::{Path, PathBuf};
use std::process::Command;

struct Env {
    root: PathBuf,
    repo: PathBuf,
}

fn sh(cwd: &Path, args: &[&str]) {
    let st = Command::new("git").current_dir(cwd).args(args).status().unwrap();
    assert!(st.success(), "git {args:?}");
}

fn setup(name: &str) -> Env {
    let root = std::env::temp_dir().join(format!("axelstra-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let repo = root.join("repo");
    std::fs::create_dir_all(repo.join(".kanban")).unwrap();
    sh(&repo, &["init", "-q", "-b", "main"]);
    sh(&repo, &["config", "user.email", "t@t"]);
    sh(&repo, &["config", "user.name", "t"]);
    std::fs::write(repo.join(".gitignore"), ".kanban/\n").unwrap();
    std::fs::write(repo.join("shared.txt"), "base\n").unwrap();
    sh(&repo, &["add", "-A"]);
    sh(&repo, &["commit", "-qm", "init"]);
    Env { repo: git::canon(&repo).unwrap(), root }
}

impl Env {
    fn card(&self, id: &str, deps: &[&str], extra: &str) {
        let t = format!(
            "---\nid: {id}\nstatus: todo\ndepends_on: [{}]\n{extra}---\ntask {id}\n",
            deps.join(",")
        );
        std::fs::write(self.repo.join(".kanban").join(format!("{id}.md")), t).unwrap();
    }

    fn status(&self, id: &str) -> String {
        let t = std::fs::read_to_string(self.repo.join(".kanban").join(format!("{id}.md"))).unwrap();
        card::parse(&t, Path::new(id)).unwrap().meta.status
    }

    fn run(&self, name: &str) -> Report {
        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/fake-agent.ps1");
        let script = git::canon(&script).unwrap();
        let marker = self.root.join("flaky.marker");
        orch::run(&Options {
            repo: self.repo.clone(),
            name: name.into(),
            max_concurrency: 2,
            wt_root: self.root.join("wt"),
            agent_override: Some(
                ["powershell", "-NoProfile", "-File", &script.to_string_lossy(), "-Marker", &marker.to_string_lossy()]
                    .map(String::from)
                    .to_vec(),
            ),
            poll_ms: 100,
            runner: None,
        })
        .unwrap()
    }
}

#[test]
fn dag_merges_everything_in_order() {
    let e = setup("dag");
    e.card("a", &[], "");
    e.card("b", &["a"], "");
    e.card("c", &[], "");
    let r = e.run("t");
    assert_eq!(r.merged, ["a", "b", "c"]);
    for f in ["a.txt", "b.txt", "c.txt"] {
        assert!(r.integ_wt.join(f).exists(), "{f} missing");
    }
    assert_eq!(e.status("b"), "done");
    // task worktrees are cleaned up, only the integration one stays
    assert!(!e.root.join("wt").join("t-a").exists());
}

#[test]
fn conflict_goes_to_needs_review_and_integration_stays_clean() {
    let e = setup("conflict");
    e.card("a", &[], "");
    e.card("conflict", &[], "");
    let r = e.run("t");
    assert_eq!(r.merged.len(), 1);
    assert_eq!(r.needs_review.len(), 1);
    assert!(git::must(&r.integ_wt, &["status", "--porcelain"]).unwrap().is_empty());
    let loser = &r.needs_review[0].0;
    assert_eq!(e.status(loser), "needs-review");
    assert!(e.root.join("wt").join(format!("t-{loser}")).exists(), "worktree kept for inspection");
}

#[test]
fn hung_agent_is_killed_on_timeout_and_does_not_block_others() {
    let e = setup("timeout");
    e.card("hang", &[], "timeout_min: 0.03\n");
    e.card("c", &[], "");
    let t0 = std::time::Instant::now();
    let r = e.run("t");
    assert!(t0.elapsed().as_secs() < 30);
    assert_eq!(r.merged, ["c"]);
    assert_eq!(r.failed.len(), 1);
    assert!(r.failed[0].1.contains("timeout"), "{:?}", r.failed);
    assert_eq!(e.status("hang"), "backlog");
}

#[test]
fn failed_task_retries_with_fresh_worktree() {
    let e = setup("retry");
    e.card("flaky", &[], "max_retries: 1\n");
    let r = e.run("t");
    assert_eq!(r.merged, ["flaky"], "{:?}", r.failed);
}

#[test]
fn failed_task_without_retries_blocks_dependents_only() {
    let e = setup("nodeps");
    e.card("flaky", &[], "");
    e.card("after", &["flaky"], "");
    e.card("c", &[], "");
    let r = e.run("t");
    assert_eq!(r.merged, ["c"]);
    assert_eq!(r.blocked, ["after"]);
    assert_eq!(e.status("after"), "todo");
}

#[test]
fn rerun_rolls_back_half_finished_merge_and_resumes() {
    let e = setup("resume");
    e.card("a", &[], "");
    let r = e.run("t");
    assert_eq!(r.merged, ["a"]);

    // simulate a crash after merge_begin: integration moved past the recorded sha
    let before = git::head(&r.integ_wt).unwrap();
    let events = git::common_dir(&e.repo).unwrap().join("axelstra/orch/t/events.jsonl");
    let mut log = std::fs::read_to_string(&events).unwrap();
    log.push_str(&format!("{{\"t\":1,\"ev\":\"merge_begin\",\"task\":\"zz\",\"integ_before\":\"{before}\"}}\n"));
    std::fs::write(&events, log).unwrap();
    std::fs::write(r.integ_wt.join("half.txt"), "partial").unwrap();
    sh(&r.integ_wt, &["add", "-A"]);
    sh(&r.integ_wt, &["-c", "user.name=x", "-c", "user.email=x@x", "commit", "-qm", "half merge"]);

    e.card("b", &[], "");
    let r2 = e.run("t");
    assert_eq!(r2.merged, ["b"]);
    assert!(!r2.integ_wt.join("half.txt").exists(), "half-done merge not rolled back");
    assert!(r2.integ_wt.join("a.txt").exists(), "earlier merge lost");
}
