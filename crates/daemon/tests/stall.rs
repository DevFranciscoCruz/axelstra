mod common;
use axelstra_daemon::client::Client;
use axelstra_daemon::server;
use serde_json::{json, Value};
use std::time::{Duration, Instant};

fn shell() -> (String, Vec<String>) {
    if cfg!(windows) {
        ("cmd".into(), vec!["/q".into()])
    } else {
        ("sh".into(), vec![])
    }
}

fn spawn(c: &mut Client, label: &str, meta: Value, extra: Value) -> u64 {
    let (prog, args) = shell();
    let mut p = json!({"program": prog, "args": args, "label": label, "meta": meta});
    for (k, v) in extra.as_object().unwrap() {
        p[k] = v.clone();
    }
    c.call("session.spawn", p).unwrap()["id"].as_u64().unwrap()
}

fn row(c: &mut Client, id: u64) -> Value {
    c.call("session.list", json!({})).unwrap().as_array().unwrap().iter().find(|s| s["id"] == id).unwrap().clone()
}

fn from(c: &mut Client, boss: u64, id: u64) -> Vec<Value> {
    let inbox = c.call("msg.inbox", json!({"session": boss, "all": true})).unwrap();
    inbox.as_array().unwrap().iter().filter(|m| m["from"] == id).cloned().collect()
}

#[test]
fn a_quiet_child_is_told_to_its_parent_once_and_legitimately_quiet_sessions_never_cause_anything() {
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(90));
        eprintln!("watchdog: test stuck");
        std::process::exit(1);
    });
    common::data_dir();
    // this test binary has one test: a 3 s threshold (honoured only under AXELSTRA_TESTING)
    std::env::set_var("AXELSTRA_STALL_SECS", "3");
    let root = std::env::temp_dir().join(format!("axelstra-stall-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let pipe = format!("{}-stall-{}", common::base_pipe(), std::process::id());
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
    let mut c = Client::connect(&pipe, &token).unwrap();

    let agent = json!({"kind": "agent"});
    // a goal is a request: these are not blank sessions
    let goal = |extra: Value| {
        let mut e = json!({"goal": "do the task"});
        for (k, v) in extra.as_object().unwrap() {
            e[k] = v.clone();
        }
        e
    };
    let boss = spawn(&mut c, "boss", agent.clone(), goal(json!({})));
    let quiet = spawn(&mut c, "quiet", agent.clone(), goal(json!({"parent": boss})));
    let reported = spawn(&mut c, "reported", agent.clone(), goal(json!({"parent": boss})));
    let escalated = spawn(&mut c, "escalated", agent.clone(), goal(json!({"parent": boss})));
    let asking = spawn(&mut c, "asking", agent.clone(), goal(json!({"parent": boss})));
    let main = spawn(&mut c, "main", json!({"kind": "agent", "main": true}), goal(json!({})));
    let term = spawn(&mut c, "term", json!({"kind": "terminal"}), json!({}));
    let lonely = spawn(&mut c, "lonely", agent.clone(), goal(json!({})));
    let blank = spawn(&mut c, "blank", agent.clone(), json!({}));
    let driven = spawn(&mut c, "driven", agent.clone(), goal(json!({})));
    let waiter = spawn(&mut c, "waiter", agent.clone(), goal(json!({"parent": boss})));
    let permission = spawn(&mut c, "permission", agent.clone(), goal(json!({"parent": boss})));
    assert!(c.call("session.await", json!({"id": waiter, "reason": "  "})).unwrap_err().to_string().contains("reason"));
    c.call("session.await", json!({"id": waiter, "reason": "CI #3703", "for_min": 30})).unwrap();
    c.call("msg.send", json!({"from": reported, "to": boss, "text": "[ok] all done", "kind": "done", "quiet": true})).unwrap();
    c.call("msg.send", json!({"from": escalated, "to": boss, "text": "need help", "kind": "escalate", "quiet": true})).unwrap();
    c.call("msg.send", json!({"from": asking, "to": boss, "text": "which one?", "kind": "ask", "quiet": true})).unwrap();
    c.call("session.hint", json!({"id": permission, "state": "waiting", "kind": "permission", "text": "Allow Bash?"})).unwrap();
    // the user typed into it: the user drives it
    c.send_input(driven as u32, b"x").unwrap();
    let excused = [
        (reported, "called done"),
        (escalated, "escalated"),
        (asking, "ask pending"),
        (permission, "permission wait"),
        (waiter, "awaiting"),
        (blank, "never asked"),
        (driven, "user driven"),
        (main, "main"),
        (term, "terminal"),
        (boss, "has running children"),
    ];

    // not stalled right after the start
    assert_eq!(row(&mut c, quiet)["stalled"], false);

    // the parent gets one line from the quiet child (it has no Stop hook: the quiet threshold is enough)
    let t0 = Instant::now();
    let got = loop {
        let got = from(&mut c, boss, quiet);
        if !got.is_empty() {
            break got;
        }
        assert!(t0.elapsed() < Duration::from_secs(15), "parent never told");
        std::thread::sleep(Duration::from_millis(250));
    };
    assert_eq!(got.len(), 1);
    let text = got[0]["text"].as_str().unwrap();
    assert!(text.contains("\"quiet\" (session") && text.contains("never reported a turn") && text.contains("axelstra read"), "{text}");
    assert!(!text.contains('\n'), "one line: {text}");
    // a child with a parent agent is the parent's: the user is not alerted, `stall_parent` says the parent was told
    let q = row(&mut c, quiet);
    assert_eq!((q["stalled"].as_bool(), q["stall_parent"].as_bool()), (Some(false), Some(true)));
    // an agent with no parent agent that the user does not drive is shown to the user
    let t0 = Instant::now();
    while row(&mut c, lonely)["stalled"] != true {
        assert!(t0.elapsed() < Duration::from_secs(10), "the parentless agent was never flagged");
        std::thread::sleep(Duration::from_millis(250));
    }
    let l = row(&mut c, lonely);
    assert_eq!(l["stall"]["code"], "agent.stalled");
    assert_eq!(l["stall"]["params"]["task"], "lonely");

    // no repeat while nothing changes, even with output in between, and nothing for the sessions that must not fire
    std::thread::sleep(Duration::from_secs(4));
    assert_eq!(from(&mut c, boss, quiet).len(), 1, "told twice");
    for (id, why) in excused {
        assert!(from(&mut c, boss, id).iter().all(|m| !m["text"].as_str().unwrap_or_default().contains("(session")), "{why}");
        // a reported session may already have been closed by its done-closer (it left the list): also not stalled
        let rows = c.call("session.list", json!({})).unwrap();
        if let Some(r) = rows.as_array().unwrap().iter().find(|s| s["id"] == id) {
            assert_eq!(r["stalled"], false, "{why}");
            assert_eq!(r["stall_parent"], false, "{why}");
        }
    }

    // real work (a message to it) ends the episode: the next stall is a new one and the parent hears once more
    c.call("msg.send", json!({"to": quiet, "text": "go on", "quiet": true})).unwrap();
    let t0 = Instant::now();
    while from(&mut c, boss, quiet).len() < 2 {
        assert!(t0.elapsed() < Duration::from_secs(15), "a new episode was never told");
        std::thread::sleep(Duration::from_millis(250));
    }
    std::thread::sleep(Duration::from_secs(4));
    assert_eq!(from(&mut c, boss, quiet).len(), 2);

    // awaiting something external: shown with its reason, never stalled, the parent is not told
    let w = row(&mut c, waiter);
    assert_eq!(w["stalled"], false, "awaiting");
    assert_eq!(w["awaiting"]["reason"], "CI #3703");
    assert_eq!(w["awaiting"]["until"].as_u64().unwrap() - w["awaiting"]["since"].as_u64().unwrap(), 30 * 60_000);
    assert!(row(&mut c, quiet)["awaiting"].is_null());
    // ending it hands the session back to the watchdog: its parent hears once
    c.call("session.await", json!({"id": waiter, "clear": true})).unwrap();
    assert!(row(&mut c, waiter)["awaiting"].is_null());
    let t0 = Instant::now();
    while from(&mut c, boss, waiter).is_empty() {
        assert!(t0.elapsed() < Duration::from_secs(10), "cleared await never told");
        std::thread::sleep(Duration::from_millis(250));
    }

    // the threshold is a setting: 15 by default, 0 turns the watchdog off, nonsense is refused with a code
    assert_eq!(c.call("settings.get", json!({})).unwrap()["stall_minutes"], 15);
    c.call("settings.set", json!({"stall_minutes": 12})).unwrap();
    assert_eq!(c.call("settings.get", json!({})).unwrap()["stall_minutes"], 12);
    c.call("settings.set", json!({"stall_minutes": 0})).unwrap();
    assert_eq!(c.call("settings.get", json!({})).unwrap()["stall_minutes"], 0);
    for bad in [json!(5000), json!(-1), json!("x")] {
        let err = c.call("settings.set", json!({ "stall_minutes": bad })).unwrap_err().to_string();
        assert!(err.contains("stall_minutes"), "{err}");
    }
    assert_eq!(c.call("settings.get", json!({})).unwrap()["stall_minutes"], 0);
    let _ = std::fs::remove_dir_all(&root);
}
