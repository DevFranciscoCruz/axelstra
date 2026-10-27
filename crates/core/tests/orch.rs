//! End-to-end orchestration tests driven by tools/fake-agent.ps1 (Windows) / fake-agent.sh (Unix).

use axelstra_core::orch::{self, Options, Report};
use axelstra_core::runner::{AgentSpawn, Handle, ProcessRunner, Runner};
use axelstra_core::{card, git};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

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
        let t = format!("---\nid: {id}\nstatus: todo\ndepends_on: [{}]\n{extra}---\ntask {id}\n", deps.join(","));
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
        let o = Options {
            repo: self.repo.clone(),
            name: name.into(),
            max_concurrency: 2,
            wt_root: self.root.join("wt"),
            agent_override: Some(fake_agent(self)),
            poll_ms: 100,
            runner: None,
        };
        if self.trusted {
            orch::run_trusted(&o)
        } else {
            orch::run(&o)
        }
    }
}

/// Command line of the scripted fake agent (tools/fake-agent.*), whose behaviour is keyed on the task id.
fn fake_agent(e: &Env) -> Vec<String> {
    let tools = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools");
    let marker = e.root.join("flaky.marker");
    if cfg!(windows) {
        let script = git::canon(&tools.join("fake-agent.ps1")).unwrap();
        ["powershell", "-NoProfile", "-File", &script.to_string_lossy(), "-Marker", &marker.to_string_lossy()].map(String::from).to_vec()
    } else {
        let script = git::canon(&tools.join("fake-agent.sh")).unwrap();
        ["sh", &script.to_string_lossy(), &marker.to_string_lossy()].map(String::from).to_vec()
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
    std::fs::write(
        e.repo.join(".kanban/evil.md"),
        "---
id: x/../../y
status: todo
---
body
",
    )
    .unwrap();
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
    e.card(
        "a",
        &[],
        "acceptance: \"exit 0\"
",
    );
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
    std::fs::write(
        wt.join("shared.txt"),
        "resolved
",
    )
    .unwrap();
    sh(&wt, &["add", "-A"]);
    sh(&wt, &["-c", "user.name=x", "-c", "user.email=x@x", "commit", "-qm", "resolve"]);

    orch::retry_merge(&e.repo, wt_root, "t", &loser, false).unwrap();
    assert_eq!(e.status(&loser), "done");
    assert!(r.integ_wt.join(format!("{loser}.txt")).exists());
    assert!(!wt.exists(), "task worktree removed after the merge");
    // a card that is no longer in review cannot be merged again
    assert!(orch::retry_merge(&e.repo, wt_root, "t", &loser, false).is_err());
}

#[test]
fn invalid_timeout_cards_never_reach_the_orchestrator() {
    let e = setup("badtimeout");
    for (i, v) in ["-1", ".nan", ".inf", "1e30"].iter().enumerate() {
        e.card(&format!("bad{i}"), &[], &format!("timeout_min: {v}\n"));
    }
    let l = card::load_dir(&e.repo.join(".kanban")).unwrap();
    assert!(l.cards.is_empty());
    assert_eq!(l.broken.len(), 4);
    let err = e.try_run("t").err().expect("no runnable cards").to_string();
    assert!(err.contains("no cards"), "{err}");
    // the run left nothing "stuck": a corrected card starts right away
    e.card("ok", &[], "timeout_min: 5\n");
    assert_eq!(e.run("t").merged, ["ok"]);
}

fn run_opts(e: &Env, name: &str) -> Options {
    Options {
        repo: e.repo.clone(),
        name: name.into(),
        max_concurrency: 1,
        wt_root: e.root.join("wt"),
        agent_override: None,
        poll_ms: 100,
        runner: None,
    }
}

#[test]
fn bypass_cards_need_a_pinned_confirmation() {
    let e = setup("bypass");
    e.card("a", &[], "permission_mode: bypassPermissions\n");
    let o = run_opts(&e, "t");
    assert!(orch::run(&o).is_err());
    let err = orch::run_trusted(&o).err().expect("bypass without pin").to_string();
    assert!(err.contains("bypass") && err.contains("a: permission_mode bypassPermissions"), "{err}");
    assert_eq!(e.status("a"), "todo");

    let info = orch::trust_info(&e.repo).unwrap();
    assert_eq!(info.cards.len(), 1);
    assert!(info.cards[0].bypass && info.cards[0].permission_mode == "bypassPermissions");

    // card edited after the confirmation: refused
    e.card("a", &[], "permission_mode: bypassPermissions\nacceptance: \"exit 0\"\n");
    let err = orch::run_pinned(&o, &info.pin).err().expect("stale pin").to_string();
    assert!(err.contains("changed after"), "{err}");
    assert_eq!(e.status("a"), "todo");
}

#[test]
fn empty_agent_override_fails_the_task_without_panicking() {
    let e = setup("emptyagent");
    e.card("a", &[], "");
    let mut o = run_opts(&e, "t");
    o.agent_override = Some(vec![]);
    let r = orch::run(&o).expect("an empty agent command is a failed task, not a crashed run");
    assert!(r.merged.is_empty());
    assert_eq!(r.failed.len(), 1, "{:?}", r.failed);
    assert!(r.failed[0].1.contains("empty agent command"), "{:?}", r.failed);
    assert_eq!(e.status("a"), "backlog");
    assert!(!e.root.join("wt").join("t-a").exists(), "the half-made task worktree is cleaned up");
}

/// Runs the fake agent, except that polling the task called `bad` always errors.
struct PollFails(Arc<dyn Runner>);

struct BadHandle;

impl Handle for BadHandle {
    fn try_wait(&mut self) -> anyhow::Result<Option<String>> {
        anyhow::bail!("poll exploded")
    }
    fn kill(&mut self) {}
}

impl Runner for PollFails {
    fn spawn(&self, s: &AgentSpawn) -> anyhow::Result<Box<dyn Handle>> {
        if s.label.ends_with("/bad") {
            return Ok(Box::new(BadHandle));
        }
        self.0.spawn(s)
    }
    fn interactive(&self) -> bool {
        false
    }
}

#[test]
fn a_poll_error_fails_that_task_and_the_run_carries_on() {
    let e = setup("pollerr");
    e.card("bad", &[], "");
    e.card("c", &[], "");
    e.card("after", &["bad"], "");
    let mut o = run_opts(&e, "t");
    o.max_concurrency = 2;
    o.agent_override = Some(fake_agent(&e));
    o.runner = Some(Arc::new(PollFails(Arc::new(ProcessRunner))));
    let r = orch::run(&o).expect("one task's poll error must not abort the run");
    assert_eq!(r.merged, ["c"]);
    assert_eq!(r.failed.len(), 1, "{:?}", r.failed);
    assert_eq!(r.failed[0].0, "bad");
    assert!(r.failed[0].1.contains("poll exploded"), "{:?}", r.failed);
    assert_eq!(r.blocked, ["after"]);
    // nothing is left claiming to be in progress
    assert_eq!(e.status("bad"), "backlog");
    assert_eq!(e.status("c"), "done");
    assert_eq!(e.status("after"), "todo");
}

// ---- the run waits for events, not for a poll tick ----

/// Runs `f` on a thread and fails the test if it does not finish: a loop that waits for a tick that never comes would
/// otherwise hang the suite.
fn within<T: Send + 'static>(secs: u64, f: impl FnOnce() -> T + Send + 'static) -> T {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    rx.recv_timeout(std::time::Duration::from_secs(secs)).expect("the run did not finish: it waits for a poll tick")
}

const AN_HOUR_MS: u64 = 3_600_000;

#[test]
fn process_exits_and_results_are_events_a_huge_poll_period_changes_nothing() {
    let e = setup("evdag");
    e.card("a", &[], "");
    e.card("b", &["a"], "");
    e.card("c", &[], "");
    let mut o = run_opts(&e, "t");
    o.max_concurrency = 2;
    o.poll_ms = AN_HOUR_MS;
    o.agent_override = Some(fake_agent(&e));
    let r = within(120, move || orch::run(&o).unwrap());
    assert_eq!(r.merged, ["a", "b", "c"]);
}

#[test]
fn the_timeout_is_a_deadline_not_a_poll() {
    let e = setup("evtimeout");
    e.card("hang", &[], "timeout_min: 0.03\n");
    let mut o = run_opts(&e, "t");
    o.poll_ms = AN_HOUR_MS;
    o.agent_override = Some(fake_agent(&e));
    let r = within(120, move || orch::run(&o).unwrap());
    assert_eq!(r.failed.len(), 1);
    assert!(r.failed[0].1.contains("timeout"), "{:?}", r.failed);
}

#[cfg(unix)]
#[test]
fn result_json_ends_the_task_even_while_the_agent_keeps_running() {
    let e = setup("evsentinel");
    e.card("a", &[], "");
    let mut o = run_opts(&e, "t");
    o.poll_ms = AN_HOUR_MS;
    // an interactive agent writes its result and stays open
    o.agent_override = Some(
        ["sh", "-c", "echo x > $AXELSTRA_TASK.txt; mkdir -p .axelstra; printf '{\"status\":\"done\",\"summary\":\"ok\"}' > .axelstra/result.json; sleep 600"]
            .map(String::from)
            .to_vec(),
    );
    let t = std::time::Instant::now();
    let r = within(120, move || orch::run(&o).unwrap());
    assert_eq!(r.merged, ["a"]);
    assert!(t.elapsed() < std::time::Duration::from_secs(60), "took {:?}", t.elapsed());
}

#[cfg(unix)]
#[test]
fn an_agent_that_dies_without_a_result_is_noticed_at_once() {
    let e = setup("evdeath");
    e.card("a", &[], "");
    let mut o = run_opts(&e, "t");
    o.poll_ms = AN_HOUR_MS;
    o.agent_override = Some(["sh", "-c", "exit 0"].map(String::from).to_vec());
    let r = within(120, move || orch::run(&o).unwrap());
    assert_eq!(r.failed.len(), 1);
    assert!(r.failed[0].1.contains("without result.json"), "{:?}", r.failed);
}

/// Delegates to the real process runner but hides that it can push its exit, like a runner that has to be asked.
struct NoPush(Arc<dyn Runner>);
struct NoPushHandle(Box<dyn Handle>);

impl Handle for NoPushHandle {
    fn try_wait(&mut self) -> anyhow::Result<Option<String>> {
        self.0.try_wait()
    }
    fn kill(&mut self) {
        self.0.kill()
    }
}

impl Runner for NoPush {
    fn spawn(&self, s: &AgentSpawn) -> anyhow::Result<Box<dyn Handle>> {
        Ok(Box::new(NoPushHandle(self.0.spawn(s)?)))
    }
    fn interactive(&self) -> bool {
        false
    }
}

#[test]
fn a_runner_that_cannot_push_is_asked_again_on_a_backoff_capped_by_poll_ms() {
    let e = setup("evnopush");
    e.card("a", &[], "");
    e.card("c", &[], "");
    let mut o = run_opts(&e, "t");
    o.max_concurrency = 2;
    o.poll_ms = 200;
    o.agent_override = Some(fake_agent(&e));
    o.runner = Some(Arc::new(NoPush(Arc::new(ProcessRunner))));
    let r = within(120, move || orch::run(&o).unwrap());
    assert_eq!(r.merged, ["a", "c"]);
}
