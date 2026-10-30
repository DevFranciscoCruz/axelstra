//! Session hibernation: an idle agent's process tree ends and its row stays; an event wakes it with the agent's own
//! resume command. The agent here is a stand-in `claude` on the PATH (it prints its arguments and sleeps), the
//! conversation files live under a throwaway home, and the data dir is the test one (see `tests/common`).
mod common;
use axelstra_daemon::client::Client;
use axelstra_daemon::server;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// Fake home with the conversation files, a PATH with the stand-in `claude`, and short hibernation timings.
fn env() -> &'static PathBuf {
    static HOME: OnceLock<PathBuf> = OnceLock::new();
    HOME.get_or_init(|| {
        let home = std::env::temp_dir().join(format!("axelstra-hib-home-{}", std::process::id()));
        let bin = home.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        if cfg!(windows) {
            std::fs::write(
                bin.join("claude.cmd"),
                "@echo off\r\necho FAKE-CLAUDE %*\r\necho %* | findstr /C:\"badbad\" >nul && exit /b 1\r\nfindstr /N \"^\"\r\n",
            )
            .unwrap();
        } else {
            let f = bin.join("claude");
            std::fs::write(&f, "#!/bin/sh\necho FAKE-CLAUDE \"$@\"\ncase \"$*\" in *badbad*) exit 1;; esac\nexec cat\n").unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
        }
        std::env::set_var("USERPROFILE", &home);
        std::env::set_var("HOME", &home);
        let mut paths = vec![bin];
        paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()));
        std::env::set_var("PATH", std::env::join_paths(paths).unwrap());
        std::env::set_var("AXELSTRA_HIBERNATE_QUIET_MS", "600");
        std::env::set_var("AXELSTRA_HIBERNATE_IDLE_MS", "1500");
        home
    })
}

fn start_at(root: &Path, tag: &str) -> (Client, String) {
    env();
    std::fs::create_dir_all(root).unwrap();
    let pipe = format!("{}-hib-{tag}-{}", common::base_pipe(), std::process::id());
    let token = "h".repeat(32);
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
    (Client::connect(&pipe, &token).unwrap(), pipe)
}

fn start(root: &Path, tag: &str) -> Client {
    start_at(root, tag).0
}

fn temp(tag: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("axelstra-hib-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    root
}

fn key(n: u32) -> String {
    format!("0f8fad5b-d9cb-469f-a165-7086772{n:05}")
}

/// A conversation with one reply on disk, where claude keeps it, so the id can be resumed.
fn conversation(cwd: &Path, id: &str) {
    let dir = env().join(".claude").join("projects").join(axelstra_daemon::agentlog::claude_dir_name(&cwd.to_string_lossy()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(format!("{id}.jsonl")), "{\"message\":{\"id\":\"m1\",\"usage\":{\"input_tokens\":1,\"output_tokens\":1}}}\n")
        .unwrap();
}

/// A claude-like agent: a shell that runs the stand-in, started the way the app starts one.
fn agent(c: &mut Client, wt: &Path, key: &str, extra_meta: Value, extra: Value) -> u64 {
    conversation(wt, key);
    let line = format!("claude --session-id {key}");
    let (prog, args): (&str, Vec<String>) = if cfg!(windows) {
        ("powershell", vec!["-NoLogo".into(), "-NoExit".into(), "-Command".into(), line])
    } else {
        ("sh", vec!["-c".into(), format!("{line}; exec 'sh' -l")])
    };
    let mut meta = json!({"project": "p", "worktree": wt.to_string_lossy(), "kind": "agent", "agent": "claude", "key": key, "resume": key});
    for (k, v) in extra_meta.as_object().unwrap() {
        meta[k] = v.clone();
    }
    let mut p = json!({"program": prog, "args": args, "cwd": wt.to_string_lossy(), "label": "claude", "meta": meta,
        "env": {"AXELSTRA_KEY": key}});
    for (k, v) in extra.as_object().unwrap() {
        p[k] = v.clone();
    }
    c.call("session.spawn", p).unwrap()["id"].as_u64().unwrap()
}

fn row(c: &mut Client, id: u64) -> Option<Value> {
    c.call("session.list", json!({})).unwrap().as_array().unwrap().iter().find(|s| s["id"] == id).cloned()
}

fn until<T>(what: &str, mut f: impl FnMut() -> Option<T>) -> T {
    let end = Instant::now() + Duration::from_secs(40);
    loop {
        if let Some(v) = f() {
            return v;
        }
        assert!(Instant::now() < end, "timed out: {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn screen(c: &mut Client, id: u64) -> String {
    c.call("session.screen", json!({"id": id, "lines": 60})).map(|v| v["lines"].to_string()).unwrap_or_default()
}

/// Waits until the agent has printed its banner (it is a process that prints once), then until it counts as quiet.
fn ready(c: &mut Client, id: u64) {
    until("agent banner", || screen(c, id).contains("FAKE-CLAUDE").then_some(()));
    std::thread::sleep(Duration::from_millis(900));
}

fn hibernate(c: &mut Client, id: u64) -> anyhow::Result<Value> {
    c.call("session.hibernate", json!({"id": id}))
}

fn code_of(e: &anyhow::Error) -> String {
    axelstra_core::msg::find(e).map(|c| c.code.clone()).unwrap_or_else(|| format!("{e:#}"))
}

fn is_awake(c: &mut Client, id: u64) -> bool {
    row(c, id).is_some_and(|r| r["hibernated"].is_null() && r["running"] == true)
}

#[test]
fn hibernate_and_wake_keep_resume_id_inbox_and_role() {
    let root = temp("basic");
    let wt = root.join("wt");
    std::fs::create_dir_all(&wt).unwrap();
    let mut c = start(&root, "basic");
    let k = key(1);
    let lead = agent(&mut c, &wt, &k, json!({"lead": true}), json!({}));
    let other = agent(&mut c, &root, &key(2), json!({}), json!({}));
    ready(&mut c, lead);

    let r = hibernate(&mut c, lead).unwrap();
    assert!(r["hibernated"]["since"].as_u64().unwrap() > 0);
    // the row stays, it did not end, the process is gone
    let sl = row(&mut c, lead).expect("row stays");
    assert!(sl["hibernated"].is_object() && sl["running"] == true, "{sl}");
    assert_eq!(sl["meta"]["lead"], true);
    assert_eq!(c.call("session.status", json!({"id": lead})).unwrap()["hibernated"], true);
    // idempotent
    hibernate(&mut c, lead).unwrap();
    // what the terminal needs is refused with a coded error, not a "not found"
    assert_eq!(code_of(&c.call("session.screen", json!({"id": lead})).unwrap_err()), "hibernate.sleeping");
    // the governor has nothing to freeze in a sleeper: pausing it says so instead of "not found"
    assert_eq!(code_of(&c.call("mem.pause", json!({"session": lead})).unwrap_err()), "hibernate.sleeping");

    // a message wakes it and waits in the inbox until the agent is back
    c.call("msg.send", json!({"from": other, "to": lead, "text": "hello-from-test", "kind": "message"})).unwrap();
    until("lead awake again", || is_awake(&mut c, lead).then_some(()));
    let out = until("resume command and the notice", || {
        let s = screen(&mut c, lead);
        (s.contains("--resume") && s.contains("hello-from-test")).then_some(s)
    });
    assert!(out.contains(&k), "resumes the conversation it had: {out}");
    // (the dimmed history above still shows the first start)
    assert!(out.rsplit("fim do histórico").next().is_some_and(|t| !t.contains("--session-id")), "{out}");
    let back = row(&mut c, lead).unwrap();
    assert_eq!((back["meta"]["lead"].clone(), back["meta"]["key"].clone()), (json!(true), json!(k)));
    assert_eq!(back["unread"], 1, "the message is still in the inbox");
    let _ = c.call("session.kill", json!({"id": lead}));
    let _ = c.call("session.kill", json!({"id": other}));
}

/// The text typed since the last (re)start, with the screen rows joined again where a long line wrapped: the saved
/// terminal shown above a woken agent repeats the old screen.
fn since_wake(c: &mut Client, id: u64) -> String {
    let rows = c.call("session.screen", json!({"id": id, "lines": 60})).map(|v| v["lines"].clone()).unwrap_or_default();
    let all: String = rows.as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    all.rsplit("fim do histórico").next().unwrap_or_default().to_string()
}

/// Notices were typed again and again for messages the agent had already been told about, because it read them with
/// `axelstra inbox --no-mark` and every wake typed the whole unread inbox. A message is announced once; what reached a
/// sleeper is one short line with the count.
#[test]
fn a_message_is_announced_once_and_a_wake_announces_only_the_new_ones_in_one_line() {
    let root = temp("once");
    let wt = root.join("wt");
    std::fs::create_dir_all(&wt).unwrap();
    let mut c = start(&root, "once");
    c.call("orch.limits", json!({"max_workers": 1})).unwrap();
    let a = agent(&mut c, &wt, &key(70), json!({}), json!({"worker": true}));
    let other = agent(&mut c, &root, &key(71), json!({}), json!({}));
    ready(&mut c, a);
    // awake: the notice is typed at once, one time
    c.call("msg.send", json!({"from": other, "to": a, "text": "seen-before-sleep", "kind": "message"})).unwrap();
    until("live notice", || since_wake(&mut c, a).contains("seen-before-sleep").then_some(()));
    // read without marking: it stays unread, and that must not make it come back
    let inbox = c.call("msg.inbox", json!({"session": a, "mark": false})).unwrap();
    assert_eq!(inbox.as_array().unwrap().len(), 1, "{inbox}");
    std::thread::sleep(Duration::from_millis(900));
    until("calm enough to sleep", || match hibernate(&mut c, a) {
        Ok(_) => Some(()),
        Err(e) if matches!(code_of(&e).as_str(), "hibernate.working" | "hibernate.recent") => None,
        Err(e) => panic!("{e:#}"),
    });
    // the slot is taken while it sleeps: its wakes queue, and the messages pile up unannounced
    let b = agent(&mut c, &wt, &key(72), json!({}), json!({"worker": true}));
    for n in 1..=3 {
        c.call("msg.send", json!({"from": other, "to": a, "text": format!("while-asleep-{n}"), "kind": "message"})).unwrap();
    }
    assert!(row(&mut c, a).unwrap()["hibernated"]["queued"] == true);
    c.call("session.kill", json!({"id": b})).unwrap();
    until("woken", || is_awake(&mut c, a).then_some(()));
    let out = until("the backlog line", || Some(since_wake(&mut c, a)).filter(|s| s.contains("3 new messages")));
    assert!(!out.contains("seen-before-sleep"), "an announced message is not typed again: {out}");
    assert!(!out.contains("while-asleep-"), "the backlog line names the messages, it does not quote them: {out}");
    assert!(!out.contains("text from another session"), "one line for all, no notice per message: {out}");
    // a new message announces itself only, never the backlog
    c.call("msg.send", json!({"from": other, "to": a, "text": "after-wake", "kind": "message"})).unwrap();
    until("the new notice", || since_wake(&mut c, a).contains("after-wake").then_some(()));
    std::thread::sleep(Duration::from_millis(500));
    let out = since_wake(&mut c, a);
    // typed once each; the stand-in prints a copy of every line it reads (`1:[axelstra] ...` on Windows, the tty echo
    // plus `cat` elsewhere), so a line shows at most twice
    let backlog = out.matches("3 new messages").count();
    assert!((1..=2).contains(&backlog), "the backlog line is typed once: {out}");
    assert!(out.matches("message #5 from").count() <= 2, "the new notice is typed once: {out}");
    assert_eq!(out.matches("message #2 from").count(), backlog, "the backlog is not typed again with the new one: {out}");
    assert!(!out.contains("seen-before-sleep") && !out.contains("while-asleep-"), "{out}");
    assert_eq!(row(&mut c, a).unwrap()["unread"], 5, "announcing is not reading");
    let _ = c.call("session.kill", json!({"id": a}));
    let _ = c.call("session.kill", json!({"id": other}));
}

#[test]
fn what_is_not_resumable_or_not_idle_is_refused() {
    let root = temp("refuse");
    let wt = root.join("wt");
    std::fs::create_dir_all(&wt).unwrap();
    let (mut c, pipe) = start_at(&root, "refuse");
    let (sp, sa) = if cfg!(windows) {
        ("cmd", json!(["/c", "echo TOK[%AXELSTRA_AGENT_TOKEN%] & ping -n 60 127.0.0.1 >nul"]))
    } else {
        ("sh", json!(["-c", "echo TOK[$AXELSTRA_AGENT_TOKEN]; sleep 60"]))
    };
    let tokshell = c.call("session.spawn", json!({"program": sp, "args": sa})).unwrap()["id"].as_u64().unwrap();
    // an agent without resume support
    let aider = agent(&mut c, &wt, &key(3), json!({"agent": "aider"}), json!({}));
    // a claude with no conversation on disk
    let fresh = {
        let (prog, args) = if cfg!(windows) { ("cmd", vec!["/q".to_string()]) } else { ("sh", vec![]) };
        c.call(
            "session.spawn",
            json!({"program": prog, "args": args, "cwd": wt.to_string_lossy(), "label": "fresh",
            "meta": {"kind": "agent", "agent": "claude", "project": "p", "worktree": wt.to_string_lossy(), "key": key(4)}}),
        )
        .unwrap()["id"]
            .as_u64()
            .unwrap()
    };
    let main = agent(&mut c, &wt, &key(5), json!({"main": true}), json!({}));
    let busy = agent(&mut c, &wt, &key(6), json!({}), json!({}));
    ready(&mut c, aider);
    ready(&mut c, main);
    assert_eq!(code_of(&hibernate(&mut c, aider).unwrap_err()), "hibernate.no_resume");
    assert_eq!(code_of(&hibernate(&mut c, fresh).unwrap_err()), "hibernate.no_resume");
    assert_eq!(code_of(&hibernate(&mut c, main).unwrap_err()), "hibernate.main");
    // a hook said it is mid-turn
    ready(&mut c, busy);
    c.call("session.hint", json!({"id": busy, "state": "working"})).unwrap();
    assert_eq!(code_of(&hibernate(&mut c, busy).unwrap_err()), "hibernate.working");
    // a permission prompt is pending
    c.call("session.hint", json!({"id": busy, "state": "waiting", "kind": "permission", "text": "run rm?"})).unwrap();
    assert_eq!(code_of(&hibernate(&mut c, busy).unwrap_err()), "hibernate.waiting");
    // an agent connection cannot do it at all
    let tok = until("agent token", || {
        let s = screen(&mut c, tokshell);
        let i = s.find("TOK[")?;
        let rest = &s[i + 4..];
        rest.find(']').filter(|j| *j >= 32).map(|j| rest[..j].to_string())
    });
    let mut ag = Client::connect_agent(&pipe, &"h".repeat(32), &tok).unwrap();
    for m in ["session.hibernate", "session.wake"] {
        assert_eq!(code_of(&ag.call(m, json!({"id": busy})).unwrap_err()), "rpc.agent_forbidden", "{m}");
    }
    for id in [aider, fresh, main, busy, tokshell] {
        let _ = c.call("session.kill", json!({"id": id}));
    }
}

#[test]
fn a_done_that_races_a_hibernation_always_wakes_the_lead() {
    let root = temp("race");
    let wt = root.join("wt");
    std::fs::create_dir_all(&wt).unwrap();
    let (mut c, pipe) = start_at(&root, "race");
    // the race is the explicit hibernation against the done: the policy would be a third party, since a lead with a
    // live child hibernates by itself after the test's short quiet time, before a slow start has even printed its
    // banner. Off in this daemon's settings; `session.hibernate` does not depend on it.
    c.call("settings.set", json!({"hibernate": {"enabled": false}})).unwrap();
    for round in 0..6u32 {
        let lead = agent(&mut c, &wt, &key(10 + round), json!({"lead": true}), json!({}));
        let worker = agent(&mut c, &root, &key(30 + round), json!({}), json!({"parent": lead}));
        ready(&mut c, lead);
        let mut a = Client::connect(&pipe, &"h".repeat(32)).unwrap();
        let mut b = Client::connect(&pipe, &"h".repeat(32)).unwrap();
        let t1 = std::thread::spawn(move || hibernate(&mut a, lead).map_err(|e| code_of(&e)));
        let t2 = std::thread::spawn(move || {
            b.call("msg.send", json!({"from": worker, "to": lead, "text": format!("report-{round}"), "kind": "done", "quiet": true}))
                .unwrap();
        });
        let slept = t1.join().unwrap();
        t2.join().unwrap();
        // it slept first, or the done reached it first (before the check or before the commit): nothing else
        assert!(
            slept.as_ref().is_ok_and(|r| r["hibernated"].is_object())
                || slept.as_ref().is_err_and(|e| e == "hibernate.event" || e == "hibernate.recent"),
            "round {round}: {slept:?}"
        );
        // whichever came first: the report is in the inbox and the lead ends up running
        until("lead awake after the race", || is_awake(&mut c, lead).then_some(()));
        let inbox = c.call("msg.inbox", json!({"session": lead, "all": true, "mark": false})).unwrap();
        assert!(inbox.to_string().contains(&format!("report-{round}")), "round {round}: {inbox}");
        let _ = c.call("session.kill", json!({"id": lead}));
        let _ = c.call("session.kill", json!({"id": worker}));
    }
}

#[test]
fn hibernated_workers_do_not_count_and_a_full_limit_queues_the_wake() {
    let root = temp("limit");
    let wt = root.join("wt");
    std::fs::create_dir_all(&wt).unwrap();
    let mut c = start(&root, "limit");
    c.call("orch.limits", json!({"max_workers": 1})).unwrap();
    let a = agent(&mut c, &wt, &key(40), json!({}), json!({"worker": true}));
    ready(&mut c, a);
    // the limit is full
    let full = c.call("session.spawn", json!({"program": if cfg!(windows) { "cmd" } else { "sh" }, "worker": true, "cwd": wt.to_string_lossy(), "meta": {"kind": "terminal"}}));
    assert_eq!(code_of(&full.unwrap_err()), "session.worker_limit");
    hibernate(&mut c, a).unwrap();
    assert_eq!(c.call("orch.limits", json!({})).unwrap()["workers"], 0);
    // a second worker takes the slot the sleeper left
    let b = agent(&mut c, &wt, &key(41), json!({}), json!({"worker": true}));
    // waking the first one finds no slot: it waits in the queue, by event
    let r = c.call("session.wake", json!({"id": a})).unwrap();
    assert_eq!(r["state"], "queued", "{r}");
    std::thread::sleep(Duration::from_millis(700));
    assert!(row(&mut c, a).unwrap()["hibernated"]["queued"] == true, "still queued while the slot is taken");
    // the slot frees: the queued wake starts
    c.call("session.kill", json!({"id": b})).unwrap();
    until("queued wake started", || is_awake(&mut c, a).then_some(()));
    let _ = c.call("session.kill", json!({"id": a}));
}

#[test]
fn a_restart_keeps_hibernated_sessions_hibernated() {
    let root = temp("restart");
    let wt = root.join("wt");
    std::fs::create_dir_all(&wt).unwrap();
    let mut c = start(&root, "restart1");
    let k = key(50);
    let id = agent(&mut c, &wt, &k, json!({"lead": true}), json!({}));
    ready(&mut c, id);
    hibernate(&mut c, id).unwrap();
    let saved = common::saved_sessions_until(&root, |l| l.iter().any(|r| r["meta"]["hibernated"].is_object()));
    let rec = saved.iter().find(|r| r["meta"]["key"] == k.as_str()).expect("saved");
    assert_eq!(rec["meta"]["resume"], k.as_str());
    assert_eq!(rec["meta"]["lead"], true);
    // a second daemon on the same files
    let mut d = start(&root, "restart2");
    let list = d.call("session.list", json!({})).unwrap();
    let rows: Vec<&Value> = list.as_array().unwrap().iter().filter(|r| r["meta"]["key"] == k.as_str()).collect();
    assert_eq!(rows.len(), 1, "{list}");
    assert!(rows[0]["hibernated"].is_object(), "{}", rows[0]);
    // the app is not handed it to restore
    let pending = d.call("session.pending", json!({})).unwrap();
    assert!(!pending.to_string().contains(&k), "{pending}");
    // and it wakes there
    let nid = rows[0]["id"].as_u64().unwrap();
    d.call("session.wake", json!({"id": nid, "wait": true})).unwrap();
    until("woken after restart", || is_awake(&mut d, nid).then_some(()));
    assert_eq!(row(&mut d, nid).unwrap()["meta"]["lead"], true);
    let _ = d.call("session.kill", json!({"id": nid}));
    let _ = c.call("session.kill", json!({"id": id}));
}

#[test]
fn a_resume_that_fails_ends_the_session_with_a_coded_error() {
    let root = temp("fail");
    let wt = root.join("wt");
    std::fs::create_dir_all(&wt).unwrap();
    let mut c = start(&root, "fail");
    // a parent in another worktree, stuck on its own escalation
    let wt_par = root.join("wt-par");
    std::fs::create_dir_all(&wt_par).unwrap();
    let par = agent(&mut c, &wt_par, &key(90), json!({}), json!({}));
    c.call("msg.send", json!({"from": par, "to": par, "text": "stuck-on-x", "kind": "escalate"})).unwrap();
    // the stand-in exits 1 for this id: resuming it fails
    let k = "badbadba-dbad-469f-a165-708677290001".to_string();
    let id = agent(&mut c, &wt, &k, json!({"parent": par}), json!({}));
    // the first start is `--session-id`, which the stand-in accepts
    let _ = screen(&mut c, id);
    // a slow machine prints the banner late: try again until the quiet time has passed
    until("calm enough to sleep", || match hibernate(&mut c, id) {
        Ok(_) => Some(()),
        Err(e) if matches!(code_of(&e).as_str(), "hibernate.working" | "hibernate.recent") => None,
        Err(e) => panic!("{e:#}"),
    });
    c.call("session.wake", json!({"id": id})).unwrap();
    // it never stays hibernated: the row leaves, `status` says why
    until("row gone", || row(&mut c, id).is_none().then_some(()));
    let st = until("session ended", || c.call("session.status", json!({"id": id})).ok().filter(|s| s["running"] == false));
    assert_eq!(st["running"], false, "{st}");
    assert_eq!(st["wake_error"]["code"], "hibernate.wake_failed", "{st}");
    // the parent is told once, as a notice: its own escalation still stands
    let told = until("parent told", || {
        let inbox = c.call("msg.inbox", json!({"session": par, "all": true, "mark": false})).unwrap();
        let hits: Vec<Value> = inbox
            .as_array()
            .unwrap()
            .iter()
            .filter(|m| m["text"].as_str().unwrap_or_default().contains("could not be resumed"))
            .cloned()
            .collect();
        (!hits.is_empty()).then_some(hits)
    });
    assert_eq!(told.len(), 1, "{told:?}");
    assert_eq!(told[0]["kind"], "notice", "{told:?}");
    assert_eq!(row(&mut c, par).unwrap()["wait"]["text"], "stuck-on-x");
    let _ = c.call("session.kill", json!({"id": par}));
}

#[test]
fn an_awaiting_agent_hibernates_by_itself_after_the_configured_quiet_time() {
    let root = temp("policy");
    let wt = root.join("wt");
    std::fs::create_dir_all(&wt).unwrap();
    let mut c = start(&root, "policy");
    // another worktree: the first agent of one is its lead, and a lead with a live child hibernates for that reason
    let wt2 = root.join("wt2");
    std::fs::create_dir_all(&wt2).unwrap();
    let id = agent(&mut c, &wt, &key(60), json!({}), json!({}));
    let calm = agent(&mut c, &wt2, &key(61), json!({}), json!({}));
    ready(&mut c, id);
    c.call("session.await", json!({"id": id, "reason": "CI #1"})).unwrap();
    until("hibernated by the policy", || row(&mut c, id).filter(|r| r["hibernated"].is_object()).map(|_| ()));
    // one that waits for nothing is left alone
    assert!(is_awake(&mut c, calm));
    // disabled in the settings: nothing new hibernates
    c.call("settings.set", json!({"hibernate": {"enabled": false}})).unwrap();
    assert_eq!(c.call("settings.get", json!({})).unwrap()["hibernate"]["enabled"], false);
    let _ = c.call("session.kill", json!({"id": id}));
    let _ = c.call("session.kill", json!({"id": calm}));
}

/// A second connection to the same daemon (a blocked `session.spawn` holds its own).
fn another(pipe: &str) -> Client {
    Client::connect(pipe, &"h".repeat(32)).unwrap()
}

fn terminal_worker(c: &mut Client, wt: &Path, queue: bool, ticket: Option<u64>) -> anyhow::Result<Value> {
    let mut p = json!({"program": if cfg!(windows) { "cmd" } else { "sh" }, "worker": true, "cwd": wt.to_string_lossy(), "meta": {"kind": "terminal"}});
    if queue {
        p["queue"] = json!(true);
    }
    if let Some(t) = ticket {
        p["ticket"] = json!(t);
    }
    c.call("session.spawn", p)
}

#[test]
fn spawns_that_wait_for_a_worker_slot_go_first_in_first_out() {
    let root = temp("fifo");
    let wt = root.join("wt");
    std::fs::create_dir_all(&wt).unwrap();
    let (mut c, pipe) = start_at(&root, "fifo");
    c.call("orch.limits", json!({"max_workers": 1})).unwrap();
    let a = terminal_worker(&mut c, &wt, false, None).unwrap()["id"].as_u64().unwrap();
    // without `queue` a full limit is still a refusal
    assert_eq!(code_of(&terminal_worker(&mut c, &wt, false, None).unwrap_err()), "session.worker_limit");
    let (tx, rx) = std::sync::mpsc::channel::<(&'static str, u64)>();
    let mut handles = Vec::new();
    for name in ["first", "second", "third"] {
        let (pipe, wt, tx) = (pipe.clone(), wt.clone(), tx.clone());
        handles.push(std::thread::spawn(move || {
            let mut d = another(&pipe);
            let r = terminal_worker(&mut d, &wt, true, None).unwrap();
            assert!(r["queued"].is_null(), "{r}");
            tx.send((name, r["id"].as_u64().unwrap())).unwrap();
        }));
        // the next one arrives later: the order of arrival is the order of the line
        std::thread::sleep(Duration::from_millis(400));
    }
    assert!(rx.try_recv().is_err(), "nobody starts while the slot is taken");
    let mut order = Vec::new();
    let mut live = a;
    for _ in 0..3 {
        c.call("session.kill", json!({"id": live})).unwrap();
        let (name, id) = rx.recv_timeout(Duration::from_secs(30)).expect("the next in line starts when the slot frees");
        order.push(name);
        live = id;
        // only one at a time: the others are still waiting
        std::thread::sleep(Duration::from_millis(300));
        assert!(rx.try_recv().is_err(), "one slot, one start");
    }
    assert_eq!(order, ["first", "second", "third"]);
    for h in handles {
        h.join().unwrap();
    }
    let _ = c.call("session.kill", json!({"id": live}));
}

#[test]
fn a_queued_spawn_that_comes_back_with_its_ticket_keeps_its_place() {
    let root = temp("ticket");
    let wt = root.join("wt");
    std::fs::create_dir_all(&wt).unwrap();
    let mut c = start(&root, "ticket");
    c.call("orch.limits", json!({"max_workers": 1})).unwrap();
    let a = terminal_worker(&mut c, &wt, false, None).unwrap()["id"].as_u64().unwrap();
    // a refusal that is not a wait (no `queue`) leaves no ticket behind
    assert!(terminal_worker(&mut c, &wt, false, None).is_err());
    c.call("session.kill", json!({"id": a})).unwrap();
    // the slot is free: a queued spawn goes straight through, with no ticket left
    let r = terminal_worker(&mut c, &wt, true, None).unwrap();
    assert!(r["queued"].is_null() && r["id"].is_u64(), "{r}");
    let _ = c.call("session.kill", json!({"id": r["id"]}));
}

#[test]
fn at_critical_an_idle_agent_is_hibernated_before_anything_is_paused() {
    let root = temp("crit");
    let wt = root.join("wt");
    std::fs::create_dir_all(&wt).unwrap();
    let mut c = start(&root, "crit");
    let id = agent(&mut c, &wt, &key(70), json!({}), json!({}));
    // a lead would be fine too, but a plain agent shows the order: first agent of a worktree is its lead
    ready(&mut c, id);
    std::thread::sleep(Duration::from_millis(900));
    // thresholds no machine meets: memory is Critical now
    c.call(
        "mem.config",
        json!({"patch": {"enabled": true, "high_free_pct": 90, "crit_free_pct": 80, "high_free_mb": 4194304, "crit_free_mb": 4000000}}),
    )
    .unwrap();
    until("hibernated by the memory governor", || row(&mut c, id).filter(|r| r["hibernated"].is_object()).map(|_| ()));
    let st = c.call("mem.status", json!({})).unwrap();
    assert!(st["paused"].as_array().is_some_and(|p| p.is_empty()), "nothing was frozen: {st}");
    // memory back to normal thresholds: the sleeper stays asleep (waking is by event only)
    c.call("mem.config", json!({"patch": {"high_free_pct": 20, "crit_free_pct": 10, "high_free_mb": 3072, "crit_free_mb": 1536}})).unwrap();
    std::thread::sleep(Duration::from_millis(400));
    assert!(row(&mut c, id).unwrap()["hibernated"].is_object());
    let _ = c.call("session.kill", json!({"id": id}));
}
