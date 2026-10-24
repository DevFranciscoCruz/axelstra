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

#[test]
fn quiet_agent_is_stalled_and_its_parent_hears_once_but_no_false_alarms() {
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
    std::thread::sleep(Duration::from_millis(300));
    let mut c = Client::connect(&pipe, &token).unwrap();

    let agent = json!({"kind": "agent"});
    let boss = spawn(&mut c, "boss", agent.clone(), json!({}));
    let quiet = spawn(&mut c, "quiet", agent.clone(), json!({"parent": boss}));
    let reported = spawn(&mut c, "reported", agent.clone(), json!({"parent": boss}));
    let escalated = spawn(&mut c, "escalated", agent.clone(), json!({"parent": boss}));
    let main = spawn(&mut c, "main", json!({"kind": "agent", "main": true}), json!({}));
    let term = spawn(&mut c, "term", json!({"kind": "terminal"}), json!({}));
    let lonely = spawn(&mut c, "lonely", agent.clone(), json!({}));
    c.call("msg.send", json!({"from": reported, "to": boss, "text": "[ok] all done", "kind": "done", "quiet": true})).unwrap();
    c.call("msg.send", json!({"from": escalated, "to": boss, "text": "need help", "kind": "escalate", "quiet": true})).unwrap();
    let inbox_before = c.call("msg.inbox", json!({"session": boss})).unwrap().as_array().unwrap().len();
    assert_eq!(inbox_before, 2);

    // not stalled right after the start
    assert_eq!(row(&mut c, quiet)["stalled"], false);

    let t0 = Instant::now();
    while row(&mut c, quiet)["stalled"] != true {
        assert!(t0.elapsed() < Duration::from_secs(30), "never stalled");
        std::thread::sleep(Duration::from_millis(250));
    }
    let q = row(&mut c, quiet);
    assert_eq!(q["stall"]["code"], "agent.stalled");
    assert_eq!(q["stall"]["params"]["task"], "quiet");
    assert!(q["stall"]["text"].as_str().unwrap().contains("stalled"));
    // an agent with no parent is flagged too (it started a moment later: wait for its own threshold)
    let t0 = Instant::now();
    while row(&mut c, lonely)["stalled"] != true {
        assert!(t0.elapsed() < Duration::from_secs(10), "the parentless agent was never flagged");
        std::thread::sleep(Duration::from_millis(250));
    }

    // the parent got one message from it
    let t0 = Instant::now();
    let got = loop {
        let inbox = c.call("msg.inbox", json!({"session": boss})).unwrap();
        let from_quiet: Vec<_> = inbox.as_array().unwrap().iter().filter(|m| m["from"] == quiet).cloned().collect();
        if !from_quiet.is_empty() {
            break from_quiet;
        }
        assert!(t0.elapsed() < Duration::from_secs(15), "parent never told");
        std::thread::sleep(Duration::from_millis(250));
    };
    assert_eq!(got.len(), 1);
    assert!(got[0]["text"].as_str().unwrap().contains("stalled for"));
    assert!(got[0]["text"].as_str().unwrap().contains("axelstra msg"));

    // no repeat, and nothing for the sessions that must not fire
    std::thread::sleep(Duration::from_secs(2));
    let again = c.call("msg.inbox", json!({"session": boss})).unwrap();
    assert!(again.as_array().unwrap().iter().all(|m| m["from"] != quiet), "told twice: {again}");
    assert!(again.as_array().unwrap().iter().all(|m| m["from"] != reported && m["from"] != escalated));
    for (id, why) in
        [(reported, "called done"), (escalated, "escalated"), (main, "main"), (term, "terminal"), (boss, "has running children")]
    {
        assert_eq!(row(&mut c, id)["stalled"], false, "{why}");
    }

    // the threshold is a setting: 5 by default, 0 turns the watchdog off, nonsense is refused with a code
    assert_eq!(c.call("settings.get", json!({})).unwrap()["stall_minutes"], 5);
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
