//! Orchestration commands: messages between sessions, ask/reply, done/escalate, the worker limit and
//! `flow`, a task graph runner. Everything goes through the daemon, so the app shows what happens.

use super::ctl::{find_worktree, one_shot_line, spawn_agent, with_reap};
use super::Cmd;
use anyhow::{bail, Context, Result};
use axelstra_daemon::client::{Client, Frame};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

fn env_id(name: &str) -> Option<u32> {
    std::env::var(name).ok().and_then(|s| s.trim().parse().ok())
}

fn me() -> Result<u32> {
    env_id("AXELSTRA_SESSION").context("$AXELSTRA_SESSION is not set: run this inside an Axelstra terminal or agent")
}

fn parent(to: Option<u32>) -> Result<u32> {
    to.or_else(|| env_id("AXELSTRA_PARENT")).context("no parent session: pass --to ID ($AXELSTRA_PARENT is not set, this session was not started as a worker)")
}

fn fmt_msg(m: &Value) -> String {
    let from = m["from"].as_u64().map_or("?".to_string(), |f| f.to_string());
    let re = m["reply_to"].as_u64().map_or(String::new(), |r| format!(" (reply to #{r})"));
    format!("#{} {} from session {}{}:\n{}", m["id"], m["kind"].as_str().unwrap_or("message"), from, re, m["text"].as_str().unwrap_or_default())
}

pub fn run(cmd: Cmd, mut c: Client) -> Result<()> {
    match cmd {
        Cmd::Msg { to, text, from, quiet } => {
            let from = from.or_else(|| env_id("AXELSTRA_SESSION"));
            let r = c.call("msg.send", json!({"from": from, "to": to, "text": text, "kind": "message", "quiet": quiet}))?;
            println!("{}", r["id"]);
        }
        Cmd::Inbox { all, no_mark, json, session } => {
            let s = match session {
                Some(s) => s,
                None => me()?,
            };
            let list = c.call("msg.inbox", json!({"session": s, "all": all, "mark": !no_mark}))?;
            let list = list.as_array().cloned().unwrap_or_default();
            if json {
                println!("{}", serde_json::to_string_pretty(&list)?);
            } else if list.is_empty() {
                println!("no messages");
            }
            if !json {
                for m in &list {
                    println!("{}\n", fmt_msg(m));
                }
            }
        }
        Cmd::Ask { to, text, timeout } => {
            let from = me()?;
            let q = c.call("msg.send", json!({"from": from, "to": to, "text": text, "kind": "ask"}))?["id"].as_u64().unwrap_or(0);
            let t0 = Instant::now();
            loop {
                let r = c.call("msg.reply_of", json!({"id": q}))?;
                if let Some(text) = r["reply"]["text"].as_str() {
                    println!("{text}");
                    return Ok(());
                }
                if t0.elapsed() >= Duration::from_secs(timeout) {
                    eprintln!("no answer after {timeout}s (question #{q} is still in session {to}'s inbox)");
                    std::process::exit(2);
                }
                std::thread::sleep(Duration::from_millis(500));
            }
        }
        Cmd::Reply { id, text } => {
            let r = c.call("msg.reply_of", json!({"id": id}))?;
            let orig = &r["original"];
            let to = orig["from"].as_u64().context("that message has no sender to reply to")?;
            // an asker blocked in `axelstra ask` picks the answer up by itself: no notice typed into its screen
            let quiet = orig["kind"] == "ask";
            let m = c.call("msg.send", json!({"from": env_id("AXELSTRA_SESSION"), "to": to, "text": text, "kind": "reply", "reply_to": id, "quiet": quiet}))?;
            println!("{}", m["id"]);
        }
        Cmd::Done { text, status, to } => {
            if !["ok", "failed", "blocked"].contains(&status.as_str()) {
                bail!("--status must be ok, failed or blocked");
            }
            let to = parent(to)?;
            let body = format!("[{status}] {text}");
            c.call("msg.send", json!({"from": me()?, "to": to, "text": body, "kind": "done"}))?;
            println!("reported to session {to}");
        }
        Cmd::Escalate { text, to } => {
            let to = parent(to)?;
            c.call("msg.send", json!({"from": me()?, "to": to, "text": text, "kind": "escalate"}))?;
            println!("escalated to session {to}; waiting for an answer in your inbox (`axelstra inbox`)");
        }
        Cmd::Limit { n } => {
            let r = c.call("orch.limits", match n {
                Some(n) => json!({"max_workers": n}),
                None => json!({}),
            })?;
            println!("max workers {} ({} running)", r["max_workers"], r["workers"]);
        }
        Cmd::Flow { file, timeout } => return flow(c, &file, timeout),
        _ => unreachable!(),
    }
    Ok(())
}

#[derive(Deserialize, Clone)]
struct Task {
    id: String,
    worktree: PathBuf,
    /// claude | codex: an agent task; without it `cmd` runs as a plain command
    agent: Option<String>,
    prompt: Option<String>,
    cmd: Option<String>,
    #[serde(default)]
    after: Vec<String>,
    /// agent tasks end when it goes quiet for this many seconds (default 20) or calls `axelstra done`
    idle_secs: Option<u64>,
    lines: Option<usize>,
}

#[derive(Deserialize)]
struct FlowFile {
    tasks: Vec<Task>,
}

#[derive(PartialEq, Clone, Copy, Debug)]
enum St {
    Pending,
    Running,
    Ok,
    Failed,
    Blocked,
}

/// What a session printed last; a fresh connection because `Client` is consumed by reading.
fn read_tail(id: u32, lines: usize) -> Result<String> {
    let mut c = Client::connect(&axelstra_daemon::proto::pipe_name(), &axelstra_daemon::proto::load_or_create_token()?)?;
    c.call("session.subscribe", json!({"id": id}))?;
    let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
    std::thread::spawn(move || {
        while let Ok(f) = c.read() {
            if let Frame::Output { data, .. } = f {
                if tx.send(data).is_err() {
                    return;
                }
            }
        }
    });
    let mut raw = Vec::new();
    if let Ok(d) = rx.recv_timeout(Duration::from_millis(1500)) {
        raw.extend(d);
        while let Ok(d) = rx.recv_timeout(Duration::from_millis(150)) {
            raw.extend(d);
        }
    }
    let text = super::ctl::strip_ansi(&String::from_utf8_lossy(&raw));
    let all: Vec<&str> = text.lines().map(str::trim_end).filter(|l| !l.trim().is_empty()).collect();
    Ok(all[all.len().saturating_sub(lines)..].join("\n"))
}

fn validate(tasks: &[Task]) -> Result<()> {
    let ids: Vec<&str> = tasks.iter().map(|t| t.id.as_str()).collect();
    for (i, t) in tasks.iter().enumerate() {
        if ids[..i].contains(&t.id.as_str()) {
            bail!("duplicate task id {:?}", t.id);
        }
        if t.agent.is_none() && t.cmd.is_none() {
            bail!("task {:?} needs an agent (+ prompt) or a cmd", t.id);
        }
        for d in &t.after {
            if !ids.contains(&d.as_str()) {
                bail!("task {:?} waits for unknown task {d:?}", t.id);
            }
        }
    }
    // a cycle would leave tasks pending forever
    let mut done: Vec<&str> = vec![];
    while done.len() < tasks.len() {
        let before = done.len();
        for t in tasks {
            if !done.contains(&t.id.as_str()) && t.after.iter().all(|d| done.contains(&d.as_str())) {
                done.push(&t.id);
            }
        }
        if done.len() == before {
            bail!("the tasks form a cycle");
        }
    }
    Ok(())
}

/// Runs a task graph: a task starts when everything it waits for succeeded, at most as many at once as the
/// worker limit allows. Prints one line per event and the output tail of each finished task.
fn flow(mut c: Client, file: &std::path::Path, timeout: u64) -> Result<()> {
    let text = std::fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?;
    let FlowFile { tasks } = serde_json::from_str(&text).context("flow file must be {\"tasks\": [...]}")?;
    validate(&tasks)?;
    let projects = c.call("project.list", json!({}))?;
    for t in &tasks {
        find_worktree(&projects, &t.worktree)?;
    }
    let mut st: HashMap<String, St> = tasks.iter().map(|t| (t.id.clone(), St::Pending)).collect();
    let mut sess: HashMap<String, u32> = HashMap::new();
    let mut worked: HashMap<String, bool> = HashMap::new();
    let mut told_waiting: HashMap<String, bool> = HashMap::new();
    let t0 = Instant::now();
    loop {
        // tasks whose dependencies failed can never run
        for t in &tasks {
            if st[&t.id] == St::Pending && t.after.iter().any(|d| matches!(st[d], St::Failed | St::Blocked)) {
                st.insert(t.id.clone(), St::Blocked);
                println!("[flow] {}: blocked (a dependency failed)", t.id);
            }
        }
        let list = c.call("session.list", json!({}))?;
        let list = list.as_array().cloned().unwrap_or_default();
        for t in &tasks {
            if st[&t.id] != St::Running {
                continue;
            }
            let id = sess[&t.id];
            let s = list.iter().find(|s| s["id"].as_u64() == Some(id as u64));
            if s.map_or(false, |s| s["idle_ms"].as_u64().unwrap_or(u64::MAX) < 3000) {
                worked.insert(t.id.clone(), true);
            }
            // outcome: Some(ok, why)
            let outcome: Option<(bool, String)> = match s {
                None => Some((true, "session closed".into())),
                Some(s) if s["finished"] == json!(true) => Some((true, "reported done".into())),
                Some(s) if s["running"] != json!(true) => {
                    let code = c.call("session.status", json!({"id": id}))?["code"].as_i64();
                    Some((code == Some(0) || (t.agent.is_some() && code.is_none()), format!("exit {}", code.map_or("?".to_string(), |c| c.to_string()))))
                }
                Some(s) if t.agent.is_some() && worked.get(&t.id) == Some(&true) && s["waiting"] != json!(true) && s["idle_ms"].as_u64().unwrap_or(0) >= t.idle_secs.unwrap_or(20) * 1000 => {
                    Some((true, "went quiet".into()))
                }
                Some(s) => {
                    if s["waiting"] == json!(true) && told_waiting.insert(t.id.clone(), true).is_none() {
                        println!("[flow] {}: waiting for an answer (session {id}); use `axelstra read {id}` and `axelstra say {id} \"…\"`", t.id);
                    }
                    None
                }
            };
            if let Some((ok, why)) = outcome {
                let tail = if s.is_some() { read_tail(id, t.lines.unwrap_or(30)).unwrap_or_default() } else { String::new() };
                println!("[flow] {}: {} ({why})", t.id, if ok { "ok" } else { "FAILED" });
                if !tail.is_empty() {
                    println!("{tail}");
                }
                println!("[flow] ---");
                st.insert(t.id.clone(), if ok { St::Ok } else { St::Failed });
                let _ = c.call("session.kill", json!({"id": id}));
                let _ = c.call("session.remove", json!({"id": id}));
            }
        }
        // start what is ready
        for t in &tasks {
            if st[&t.id] != St::Pending || !t.after.iter().all(|d| st[d] == St::Ok) {
                continue;
            }
            let started: Result<u32> = match &t.agent {
                Some(agent) => spawn_agent(&mut c, &t.worktree, agent, t.prompt.as_deref(), None, true),
                None => {
                    let (_, wt) = find_worktree(&projects, &t.worktree).map(|(p, w)| (p.clone(), w))?;
                    let (program, args) = one_shot_line(t.cmd.as_deref().unwrap_or_default());
                    let label = t.cmd.as_deref().unwrap_or_default().split_whitespace().next().unwrap_or("cmd").to_string();
                    let meta = json!({"worktree": wt, "kind": "terminal"});
                    c.call("session.spawn", with_reap(json!({"program": program, "args": args, "cwd": wt, "label": label, "meta": meta, "cols": 120, "rows": 30}), None, true))
                        .map(|r| r["id"].as_u64().unwrap_or(0) as u32)
                }
            };
            match started {
                Ok(id) => {
                    println!("[flow] {}: started (session {id})", t.id);
                    sess.insert(t.id.clone(), id);
                    st.insert(t.id.clone(), St::Running);
                }
                // the worker limit is a reason to wait, not to fail
                Err(e) if e.to_string().contains("worker limit") => break,
                Err(e) => {
                    println!("[flow] {}: could not start: {e:#}", t.id);
                    st.insert(t.id.clone(), St::Failed);
                }
            }
        }
        if st.values().all(|s| !matches!(s, St::Pending | St::Running)) {
            break;
        }
        if t0.elapsed() >= Duration::from_secs(timeout) {
            eprintln!("[flow] timeout after {timeout}s; still running: {}", tasks.iter().filter(|t| st[&t.id] == St::Running).map(|t| t.id.as_str()).collect::<Vec<_>>().join(", "));
            std::process::exit(2);
        }
        std::thread::sleep(Duration::from_millis(1000));
    }
    let bad = tasks.iter().filter(|t| matches!(st[&t.id], St::Failed | St::Blocked)).count();
    println!("[flow] finished: {} ok, {} failed or blocked", tasks.len() - bad, bad);
    if bad > 0 {
        std::process::exit(1);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(id: &str, after: &[&str]) -> Task {
        Task { id: id.into(), worktree: ".".into(), agent: None, prompt: None, cmd: Some("x".into()), after: after.iter().map(|s| s.to_string()).collect(), idle_secs: None, lines: None }
    }

    #[test]
    fn accepts_a_dag() {
        assert!(validate(&[t("a", &[]), t("b", &["a"]), t("c", &["a", "b"])]).is_ok());
    }

    #[test]
    fn rejects_cycles_unknown_and_duplicates() {
        assert!(validate(&[t("a", &["b"]), t("b", &["a"])]).is_err());
        assert!(validate(&[t("a", &["zzz"])]).is_err());
        assert!(validate(&[t("a", &[]), t("a", &[])]).is_err());
    }
}
