//! The CI-fix trigger end to end with a fake `gh` and fake sessions: opt-in, one firing per commit, cooldown, hourly
//! maximum, main excluded, audit log, and the `triggers.*` methods of a real daemon.
mod common;
use axelstra_core::git::Worktree;
use axelstra_daemon::audit::Audit;
use axelstra_daemon::checks::{Check, Status};
use axelstra_daemon::client::Client;
use axelstra_daemon::projects::Project;
use axelstra_daemon::server;
use axelstra_daemon::triggers::{self, Env, Live, ProjectTriggers, Store};
use serde_json::json;
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

struct Fake {
    now: Cell<u64>,
    wts: Vec<Worktree>,
    /// sha of the failing head per worktree path; "" = CI passes
    sha: RefCell<BTreeMap<String, String>>,
    live: Live,
    polls: Cell<u32>,
    spawned: RefCell<Vec<(String, String)>>,
    fail_spawn: bool,
}

fn wt(path: &str, branch: &str, main: bool) -> Worktree {
    Worktree { path: path.into(), branch: Some(branch.into()), head: "h".into(), is_main: main }
}

fn fake() -> Fake {
    Fake {
        now: Cell::new(100_000),
        wts: vec![wt("C:/r/main", "main", true), wt("C:/r/wt-a", "ax/a", false), wt("C:/r/wt-pr", "ax/pr-9", false)],
        sha: RefCell::new(BTreeMap::new()),
        live: Live { any: true, fix_running: false },
        polls: Cell::new(0),
        spawned: RefCell::new(vec![]),
        fail_spawn: false,
    }
}

impl Fake {
    fn fail(&self, path: &str, sha: &str) {
        self.sha.borrow_mut().insert(path.into(), sha.into());
    }
}

impl Env for Fake {
    fn now(&self) -> u64 {
        self.now.get()
    }
    fn projects(&self) -> Vec<Project> {
        vec![Project {
            id: "p".into(),
            name: "p".into(),
            path: "C:/r/main".into(),
            setup: None,
            parents: Default::default(),
            ..Default::default()
        }]
    }
    fn worktrees(&self, _: &Project) -> Vec<Worktree> {
        self.wts.clone()
    }
    fn live(&self, _: &str, _: &str) -> Live {
        self.live
    }
    fn checks(&self, wt: &Path) -> Status {
        self.polls.set(self.polls.get() + 1);
        let sha = self.sha.borrow().get(&wt.to_string_lossy().to_string()).cloned().unwrap_or_default();
        if sha.is_empty() {
            return Status { pr_state: "open".into(), ci: "pass".into(), head_sha: "ok".into(), ..Default::default() };
        }
        Status {
            pr_state: "open".into(),
            ci: "fail".into(),
            number: 7,
            head_sha: sha,
            checks: vec![
                Check { name: "build".into(), state: "fail".into(), link: "https://github.com/o/r/runs/1".into() },
                Check { name: "lint".into(), state: "pass".into(), link: String::new() },
            ],
            ..Default::default()
        }
    }
    fn spawn_fix(&self, _: &Project, worktree: &str, prompt: &str) -> anyhow::Result<u32> {
        if self.fail_spawn {
            anyhow::bail!("agent missing");
        }
        self.spawned.borrow_mut().push((worktree.into(), prompt.into()));
        Ok(40 + self.spawned.borrow().len() as u32)
    }
}

struct Rig {
    dir: PathBuf,
    store: PathBuf,
    audit: Audit,
    seen: HashSet<String>,
}

fn rig(tag: &str) -> Rig {
    let dir = std::env::temp_dir().join(format!("axelstra-trig-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let store = triggers::store_path(&dir.join("projects.json"));
    let audit = Audit::new(Some(dir.join("audit.log")));
    Rig { dir, store, audit, seen: HashSet::new() }
}

impl Rig {
    fn configure(&self, f: impl FnOnce(&mut ProjectTriggers)) {
        let mut s = Store::new();
        let mut t = ProjectTriggers { ci_fix: true, confirmed_at: Some(1), ..Default::default() };
        f(&mut t);
        s.insert("p".into(), t);
        triggers::save(&self.store, &s).unwrap();
    }
    fn tick(&mut self, env: &Fake) -> Vec<triggers::Outcome> {
        triggers::tick(env, &self.store, &self.audit, &mut self.seen)
    }
}

#[test]
fn disabled_by_default_polls_nothing_and_starts_nothing() {
    let mut r = rig("off");
    let env = fake();
    env.fail("C:/r/wt-a", "a1");
    // no file at all, then a file that was never confirmed
    assert!(r.tick(&env).is_empty());
    let mut s = Store::new();
    s.insert("p".into(), ProjectTriggers { ci_fix: true, ..Default::default() });
    triggers::save(&r.store, &s).unwrap();
    assert!(r.tick(&env).is_empty());
    assert_eq!(env.polls.get(), 0, "no gh call for a project that did not opt in");
    assert!(env.spawned.borrow().is_empty());
    assert!(r.audit.list(10, None).is_empty());
}

#[test]
fn fires_once_per_commit_in_the_failing_worktree_only() {
    let mut r = rig("once");
    r.configure(|t| t.cooldown_secs = 60);
    let env = fake();
    env.fail("C:/r/main", "b1");
    env.fail("C:/r/wt-pr", "c1");
    env.fail("C:/r/wt-a", "a1");
    let out = r.tick(&env);
    assert_eq!(out.len(), 1, "{out:?}");
    assert_eq!(
        (out[0].kind, out[0].worktree.as_str(), out[0].sha.as_str()),
        ("fired", if cfg!(any(windows, target_os = "macos")) { "c:/r/wt-a" } else { "C:/r/wt-a" }, "a1")
    );
    let spawned = env.spawned.borrow().clone();
    assert_eq!(spawned.len(), 1);
    assert_eq!(spawned[0].0, "C:/r/wt-a", "started in the failing worktree, never main or the PR worktree");
    assert!(spawned[0].1.contains("\"build\"") && !spawned[0].1.contains("\"lint\""), "{}", spawned[0].1);
    assert!(spawned[0].1.contains("PR #7") && spawned[0].1.contains("commit a1"));
    // main and the PR worktree were not even looked at
    assert_eq!(env.polls.get(), 1);

    // same commit, long after the cooldown: handled already
    env.now.set(env.now.get() + 10_000);
    assert!(r.tick(&env).is_empty());
    assert_eq!(env.spawned.borrow().len(), 1);

    // a new failing commit fires again
    env.fail("C:/r/wt-a", "a2");
    let out = r.tick(&env);
    assert_eq!((out.len(), out[0].kind), (1, "fired"));
    assert_eq!(env.spawned.borrow().len(), 2);

    // state on disk: last commit and history
    let t = &triggers::load(&r.store)["p"];
    assert_eq!(t.last[&axelstra_core::paths::norm("C:/r/wt-a")].sha, "a2");
    assert_eq!(t.history.len(), 2);

    // the audit log has both firings, from the daemon itself
    let a = r.audit.list(10, None);
    assert_eq!(a.len(), 2);
    assert!(a.iter().all(|e| e["method"] == "trigger.ci_fix" && e["session"] == 0 && e["ok"] == true));
    assert_eq!(a[0]["params"]["sha"], "a1");
    assert!(std::fs::read_to_string(r.dir.join("audit.log")).unwrap().contains("trigger.ci_fix"));
    // the app is told
    let ev = triggers::take_events();
    assert!(ev.iter().any(|e| e["kind"] == "ci_fix" && e["worktree"] == "C:/r/wt-a"));
}

#[test]
fn cooldown_holds_a_new_commit_and_is_logged_once() {
    let mut r = rig("cool");
    r.configure(|t| t.cooldown_secs = 600);
    let env = fake();
    env.fail("C:/r/wt-a", "a1");
    assert_eq!(r.tick(&env)[0].kind, "fired");
    env.fail("C:/r/wt-a", "a2");
    env.now.set(env.now.get() + 100);
    let out = r.tick(&env);
    assert_eq!((out.len(), out[0].kind), (1, "limited"));
    assert!(out[0].detail.contains("cooldown"), "{}", out[0].detail);
    // the next minute: still waiting, not logged again
    env.now.set(env.now.get() + 60);
    assert!(r.tick(&env).is_empty());
    let a = r.audit.list(10, None);
    assert_eq!(a.len(), 2);
    assert_eq!((a[1]["ok"].as_bool(), a[1]["params"]["sha"].as_str()), (Some(false), Some("a2")));
    assert!(a[1]["error"].as_str().unwrap().contains("rate limited"));
    // after the cooldown the same commit fires
    env.now.set(env.now.get() + 600);
    let out = r.tick(&env);
    assert_eq!((out.len(), out[0].kind, out[0].sha.as_str()), (1, "fired", "a2"));
    assert_eq!(env.spawned.borrow().len(), 2);
}

#[test]
fn hourly_maximum_is_per_project() {
    let mut r = rig("hour");
    r.configure(|t| {
        t.cooldown_secs = 60;
        t.max_per_hour = 2;
    });
    let mut env = fake();
    env.wts.push(wt("C:/r/wt-b", "ax/b", false));
    env.wts.push(wt("C:/r/wt-c", "ax/c", false));
    for w in ["a", "b", "c"] {
        env.fail(&format!("C:/r/wt-{w}"), "a1");
    }
    let out = r.tick(&env);
    let kinds: Vec<_> = out.iter().map(|o| o.kind).collect();
    assert_eq!(kinds, ["fired", "fired", "limited"], "{out:?}");
    assert!(out[2].detail.contains("hourly"));
    assert_eq!(env.spawned.borrow().len(), 2);
    // an hour later the third goes
    env.now.set(env.now.get() + 3601);
    let out = r.tick(&env);
    assert_eq!(out.iter().map(|o| o.kind).collect::<Vec<_>>(), ["fired"]);
    assert_eq!(env.spawned.borrow().len(), 3);
}

#[test]
fn needs_a_live_agent_and_one_fix_at_a_time() {
    let mut r = rig("live");
    r.configure(|_| {});
    let mut env = fake();
    env.fail("C:/r/wt-a", "a1");
    env.live = Live { any: false, fix_running: false };
    assert!(r.tick(&env).is_empty());
    assert_eq!(env.polls.get(), 0, "an idle worktree is not polled");
    env.live = Live { any: true, fix_running: true };
    assert!(r.tick(&env).is_empty());
    assert_eq!(env.polls.get(), 0);
    env.live = Live { any: true, fix_running: false };
    assert_eq!(r.tick(&env)[0].kind, "fired");
}

#[test]
fn a_failed_start_is_logged_and_not_retried_for_the_same_commit() {
    let mut r = rig("fail");
    r.configure(|_| {});
    let mut env = fake();
    env.fail_spawn = true;
    env.fail("C:/r/wt-a", "a1");
    let out = r.tick(&env);
    assert_eq!((out[0].kind, out[0].detail.as_str()), ("failed", "agent missing"));
    assert!(r.tick(&env).is_empty(), "no retry loop");
    let a = r.audit.list(10, None);
    assert_eq!(a.len(), 1);
    assert_eq!(a[0]["ok"], false);
}

#[test]
fn passing_or_pending_ci_never_fires() {
    let mut r = rig("pass");
    r.configure(|_| {});
    let env = fake();
    assert!(r.tick(&env).is_empty());
    assert!(env.spawned.borrow().is_empty());
}

// ---- the methods of a real daemon ----

fn start(tag: &str) -> (Client, PathBuf) {
    let root = std::env::temp_dir().join(format!("axelstra-trig-rpc-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let pipe = format!("{}-trig-{tag}-{}", axelstra_daemon::proto::pipe_name(), std::process::id());
    let token = "d".repeat(32);
    let (p, t, s) = (pipe.clone(), token.clone(), root.join("projects.json"));
    std::thread::spawn(move || {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap()
            .block_on(server::serve_with_store(p, t, s))
            .unwrap();
    });
    common::wait_ready(&pipe, &token);
    (Client::connect(&pipe, &token).unwrap(), root)
}

fn add_project(c: &mut Client, root: &Path) -> String {
    let repo = root.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    for args in [&["init", "-q"][..], &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "--allow-empty", "-m", "i"]] {
        assert!(std::process::Command::new("git").current_dir(&repo).args(args).status().unwrap().success());
    }
    c.call("project.add", json!({"path": repo})).unwrap()["id"].as_str().unwrap().to_string()
}

#[test]
fn methods_default_off_and_first_enabling_needs_the_word() {
    let (mut c, root) = start("rpc");
    let id = add_project(&mut c, &root);
    let g = c.call("triggers.get", json!({"project": id})).unwrap();
    assert_eq!((g["ci_fix"].as_bool(), g["confirmed_at"].is_null()), (Some(false), true));
    assert_eq!(g["confirm_word"], "ci_fix");
    assert!(g["template"].as_str().unwrap().contains("{checks}"));
    assert_eq!(g["limits"]["cooldown_secs"], 1800);
    let pv = c.call("triggers.preview", json!({"project": id})).unwrap();
    assert!(pv["prompt"].as_str().unwrap().contains("build (ubuntu)"));
    assert_eq!(pv["never_main"], true);

    for bad in [json!({}), json!({"confirm": "yes"}), json!({"confirm": true})] {
        let mut p = json!({"project": id, "ci_fix": true});
        p.as_object_mut().unwrap().extend(bad.as_object().unwrap().clone());
        let e = c.call("triggers.set", p).unwrap_err().to_string();
        assert!(e.contains("confirm"), "{e}");
    }
    assert_eq!(c.call("triggers.get", json!({"project": id})).unwrap()["ci_fix"], false);

    let s = c.call("triggers.set", json!({"project": id, "ci_fix": true, "confirm": "ci_fix"})).unwrap();
    assert_eq!(s["ci_fix"], true);
    assert!(s["confirmed_at"].as_u64().unwrap() > 0);
    // disabling needs nothing; enabling again after the first confirmation needs nothing either
    assert_eq!(c.call("triggers.set", json!({"project": id, "ci_fix": false})).unwrap()["ci_fix"], false);
    assert_eq!(c.call("triggers.set", json!({"project": id, "ci_fix": true})).unwrap()["ci_fix"], true);
    // limits are validated
    for bad in [json!({"cooldown_secs": 5}), json!({"max_per_hour": 0}), json!({"max_per_hour": 99}), json!({"ci_fix": "yes"})] {
        let mut p = json!({"project": id});
        p.as_object_mut().unwrap().extend(bad.as_object().unwrap().clone());
        assert!(c.call("triggers.set", p).is_err());
    }
    let s = c.call("triggers.set", json!({"project": id, "cooldown_secs": 120, "max_per_hour": 5})).unwrap();
    assert_eq!(s["limits"]["cooldown_secs"], 120);
    assert_eq!(s["limits"]["max_per_hour"], 5);
    assert_eq!(s["ci_fix"], true);
    assert!(root.join("triggers.json").exists());
    assert!(c.call("triggers.get", json!({"project": "nope"})).is_err());
    assert!(c.call("triggers.events", json!({})).unwrap()["events"].is_array());
}
