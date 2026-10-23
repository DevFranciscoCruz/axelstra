//! End-to-end orchestration tests driven by tools/fake-agent.ps1 (Windows) / fake-agent.sh (Unix).

use axelstra_core::orch::{self, Options, Report};
use axelstra_core::{card, git};
use std::path::{Path, PathBuf};
use std::process::Command;

struct Env {
    root: PathBuf,
    repo: PathBuf,
    trusted: bool,
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
    Env { repo: git::canon(&repo).unwrap(), root, trusted: false }
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
        self.try_run(name).unwrap()
    }

    fn try_run(&self, name: &str) -> anyhow::Result<Report> {
        let tools = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools");
        let marker = self.root.join("flaky.marker");
        let agent: Vec<String> = if cfg!(windows) {
            let script = git::canon(&tools.join("fake-agent.ps1")).unwrap();
            ["powershell", "-NoProfile", "-File", &script.to_string_lossy(), "-Marker", &marker.to_string_lossy()]
                .map(String::from)
                .to_vec()
        } else {
            let script = git::canon(&tools.join("fake-agent.sh")).unwrap();
            ["sh", &script.to_string_lossy(), &marker.to_string_lossy()].map(String::from).to_vec()
        };
        let o = Options {
            repo: self.repo.clone(),
            name: name.into(),
            max_concurrency: 2,
            wt_root: self.root.join("wt"),
            agent_override: Some(agent),
            poll_ms: 100,
            runner: None,
        };
        if self.trusted { orch::run_trusted(&o) } else { orch::run(&o) }
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

#[test]
fn card_ids_that_escape_the_worktree_dir_are_not_loaded() {
    let e = setup("slug");
    e.card("a", &[], "");
    std::fs::write(e.repo.join(".kanban/evil.md"), "---
id: x/../../y
status: todo
---
body
").unwrap();
    let l = card::load_dir(&e.repo.join(".kanban")).unwrap();
    assert_eq!(l.cards.len(), 1);
    assert_eq!(l.broken.len(), 1);
    let r = e.run("t");
    assert_eq!(r.merged, ["a"]);
    assert!(!e.root.join("y").exists());
    let o = Options {
        repo: e.repo.clone(),
        name: "../x".into(),
        max_concurrency: 1,
        wt_root: e.root.join("wt"),
        agent_override: None,
        poll_ms: 100,
        runner: None,
    };
    assert!(orch::run(&o).is_err());
}

#[test]
fn acceptance_commands_need_trust() {
    let mut e = setup("trust");
    e.card("a", &[], "acceptance: \"exit 0\"
");
    let err = e.try_run("t").err().expect("untrusted run must refuse").to_string();
    assert!(err.contains("not trusted") && err.contains("a: exit 0"), "{err}");
    assert_eq!(e.status("a"), "todo");
    e.trusted = true;
    assert_eq!(e.run("t2").merged, ["a"]);
}

#[test]
fn retry_merge_integrates_a_resolved_conflict() {
    let e = setup("retrymerge");
    e.card("a", &[], "");
    e.card("conflict", &[], "");
    let r = e.run("t");
    let loser = r.needs_review[0].0.clone();
    assert_eq!(r.needs_review[0].1, "merge conflict");
    let wt_root = e.root.join("wt");
    let wt = wt_root.join(format!("t-{loser}"));
    let wt_root = wt_root.as_path();

    // still conflicting: stays in review
    assert!(orch::retry_merge(&e.repo, wt_root, "t", &loser, false).is_err());
    assert_eq!(e.status(&loser), "needs-review");
    assert!(git::must(&r.integ_wt, &["status", "--porcelain"]).unwrap().is_empty());

    // the user merges the integration branch into the task branch and resolves it
    let _ = Command::new("git").current_dir(&wt).args(["merge", "orch/t"]).status().unwrap();
    std::fs::write(wt.join("shared.txt"), "resolved
").unwrap();
    sh(&wt, &["add", "-A"]);
    sh(&wt, &["-c", "user.name=x", "-c", "user.email=x@x", "commit", "-qm", "resolve"]);

    orch::retry_merge(&e.repo, wt_root, "t", &loser, false).unwrap();
    assert_eq!(e.status(&loser), "done");
    assert!(r.integ_wt.join(format!("{loser}.txt")).exists());
    assert!(!wt.exists(), "task worktree removed after the merge");
    // a card that is no longer in review cannot be merged again
    assert!(orch::retry_merge(&e.repo, wt_root, "t", &loser, false).is_err());
}
