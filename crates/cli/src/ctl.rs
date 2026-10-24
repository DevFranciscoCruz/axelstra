//! Commands that talk to the running daemon (also usable by agents inside a session).

use super::{Cmd, WtCmd};
use anyhow::{bail, Context, Result};
use axelstra_daemon::automations::{ps_quote, sh_quote};
use axelstra_daemon::client::{Client, Frame};
use serde_json::{json, Value};
use std::path::Path;
use std::time::{Duration, Instant};

pub fn run(cmd: Cmd, mut c: Client) -> Result<()> {
    match cmd {
        Cmd::Ls { json } => {
            let v = c.call("session.list", json!({}))?;
            if json {
                println!("{v}");
            } else {
                print_ls(&v);
            }
        }
        Cmd::Tail { id, secs } => {
            c.call("session.subscribe", json!({"id": id}))?;
            std::thread::spawn(move || {
                while let Ok(f) = c.read() {
                    if let Frame::Output { data, .. } = f {
                        print!("{}", String::from_utf8_lossy(&data));
                    }
                }
            });
            std::thread::sleep(Duration::from_secs(secs));
            std::process::exit(0);
        }
        Cmd::Send { id, data } => c.send_input(id, &unescape(&data)?)?,
        Cmd::Say { id, text } => {
            let mut b = b"\x1b[200~".to_vec();
            b.extend_from_slice(text.as_bytes());
            b.extend_from_slice(b"\x1b[201~\r");
            c.send_input(id, &b)?
        }
        Cmd::AddProject { path } => {
            let abs = axelstra_core::git::canon(&path)?;
            println!("{}", c.call("project.add", json!({"path": abs}))?);
        }
        Cmd::SetSetup { id, setup } => {
            c.call("project.setup", json!({"id": id, "setup": setup}))?;
        }
        Cmd::Projects => println!("{}", serde_json::to_string_pretty(&c.call("project.list", json!({}))?)?),
        Cmd::Kill { id } => {
            c.call("session.kill", json!({"id": id}))?;
        }
        Cmd::Spawn { cmd } => {
            let r = c.call("session.spawn", json!({"program": cmd[0], "args": &cmd[1..], "cwd": std::env::current_dir()?}))?;
            println!("{r}");
        }
        Cmd::Worktree { cmd: WtCmd::Create { project, name, parent } } => {
            let projects = c.call("project.list", json!({}))?;
            let pid = match find_project_by_ref(&projects, &project) {
                Some(p) => p["id"].as_str().unwrap_or_default().to_string(),
                None => bail!("no registered project matches {project:?} (see `axelstra projects`)"),
            };
            // the parent is given as a folder; the daemon knows worktrees by the path git lists
            let parent = match parent {
                Some(par) => Some(find_worktree(&projects, &par)?.1),
                None => None,
            };
            let r = c.call("worktree.create", json!({"project": pid, "name": name, "parent": parent}))?;
            println!("{}", r["path"].as_str().unwrap_or_default());
            eprintln!("note: a worktree needs a lead (\"main\") agent from the start: the first agent started there becomes it, and later agents are its tasks. Start it now: `axelstra agent <path> --prompt \"<theme goal>\" --report-to` (or use `axelstra dispatch new`, which does both).");
        }
        Cmd::Worktree { cmd: WtCmd::Rm { path } } => {
            let projects = c.call("project.list", json!({}))?;
            let (p, wt) = find_worktree(&projects, &path)?;
            c.call("worktree.remove", json!({"project": p["id"], "path": wt}))?;
        }
        Cmd::Worktree { cmd: WtCmd::Sweep { dry_run, auto } } => {
            if let Some(a) = auto {
                let r = c.call("janitor.auto", json!({"on": a == "on"}))?;
                println!("automatic sweep: {}", if r["on"] == json!(true) { "on" } else { "off" });
                return Ok(());
            }
            let r = c.call("worktree.sweep", json!({"dry_run": dry_run}))?;
            let list = r["swept"].as_array().cloned().unwrap_or_default();
            for s in &list {
                let verb = if dry_run {
                    "would remove"
                } else if s["removed"] == json!(true) {
                    "removed"
                } else {
                    "could not remove"
                };
                println!(
                    "{verb} {} ({}, {})",
                    s["path"].as_str().unwrap_or_default(),
                    s["branch"].as_str().unwrap_or_default(),
                    s["why"].as_str().unwrap_or_default()
                );
            }
            if list.is_empty() {
                println!("nothing to remove");
            }
        }
        Cmd::Agent { worktree, agent, prompt, prompt_file, report_to, when, idle_secs, lines } => {
            let prompt = resolve_prompt(prompt, prompt_file.as_deref(), &mut std::io::stdin())?;
            if std::env::var("AXELSTRA_SESSION").is_ok() {
                let projects = c.call("project.list", json!({}))?;
                if let Ok((p, wt)) = find_worktree(&projects, &worktree) {
                    let in_main = p["worktrees"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .any(|w| w["path"].as_str() == Some(wt.as_str()) && w["is_main"] == json!(true));
                    if in_main {
                        eprintln!("warning: {wt} is the project's main checkout. The main session only dispatches: it never runs workers there. Use `axelstra dispatch new <project> <name> \"<request>\"` to give the theme its own worktree and lead agent.");
                    }
                }
            }
            let reap = reap_for(report_to, when.as_deref(), "idle", idle_secs, lines, std::env::var("AXELSTRA_SESSION").ok().as_deref())?;
            println!("{}", spawn_agent(&mut c, &worktree, &agent, prompt.as_deref(), reap, false)?);
        }
        Cmd::Terminal { worktree, title, cmd, report_to, when, idle_secs, lines } => {
            let projects = c.call("project.list", json!({}))?;
            let (p, wt) = find_worktree(&projects, &worktree)?;
            let reap = reap_for(report_to, when.as_deref(), "exit", idle_secs, lines, std::env::var("AXELSTRA_SESSION").ok().as_deref())?;
            let (program, args) = match (&cmd, &reap) {
                (Some(line), Some(r)) if r["mode"] == "exit" => one_shot_line(line),
                (Some(line), _) => {
                    let (program, mut args) = default_shell();
                    let ps = program.to_ascii_lowercase().contains("powershell") || program.to_ascii_lowercase().contains("pwsh");
                    shell_run(&program, &mut args, ps, line, true);
                    (program, args)
                }
                _ => default_shell(),
            };
            let label = title.unwrap_or_else(|| "terminal".into());
            let meta = json!({"project": p["id"], "worktree": wt, "kind": "terminal"});
            let r = c.call(
                "session.spawn",
                with_reap(
                    json!({"program": program, "args": args, "cwd": wt, "label": label, "meta": meta, "cols": 120, "rows": 30}),
                    reap,
                    false,
                ),
            )?;
            println!("{}", r["id"]);
        }
        Cmd::Run { worktree, report_to, lines, command } => {
            let projects = c.call("project.list", json!({}))?;
            let (p, wt) = find_worktree(&projects, &worktree)?;
            let reap = reap_for(report_to, Some("exit"), "exit", None, lines, std::env::var("AXELSTRA_SESSION").ok().as_deref())?;
            let (program, args) = one_shot_args(&command);
            let label = command[0].rsplit(['/', '\\']).next().unwrap_or(&command[0]).to_string();
            let meta = json!({"project": p["id"], "worktree": wt, "kind": "terminal"});
            let r = c.call(
                "session.spawn",
                with_reap(
                    json!({"program": program, "args": args, "cwd": wt, "label": label, "meta": meta, "cols": 120, "rows": 30}),
                    reap,
                    false,
                ),
            )?;
            println!("{}", r["id"]);
        }
        Cmd::Read { id, lines } => {
            // the daemon replays the output through a terminal emulator; an older daemon has no such method
            if let Ok(r) = c.call("session.screen", json!({"id": id, "lines": lines})) {
                for l in r["lines"].as_array().into_iter().flatten().filter_map(|l| l.as_str()) {
                    println!("{l}");
                }
                return Ok(());
            }
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
            // the snapshot is the first frame; keep taking frames until the stream goes quiet
            let mut raw = Vec::new();
            if let Ok(d) = rx.recv_timeout(Duration::from_millis(1500)) {
                raw.extend(d);
                while let Ok(d) = rx.recv_timeout(Duration::from_millis(150)) {
                    raw.extend(d);
                }
            }
            let text = strip_ansi(&String::from_utf8_lossy(&raw));
            let all: Vec<&str> = text.lines().map(str::trim_end).collect();
            let end = all.iter().rposition(|l| !l.is_empty()).map_or(0, |i| i + 1);
            let start = end.saturating_sub(lines);
            for l in &all[start..end] {
                println!("{l}");
            }
            // dropping the connection unsubscribes
            std::process::exit(0);
        }
        Cmd::Wait { id, idle_secs, timeout } => {
            let t0 = Instant::now();
            let idle_ms = idle_secs * 1000;
            loop {
                let v = c.call("session.list", json!({}))?;
                match v.as_array().and_then(|a| a.iter().find(|s| s["id"].as_u64() == Some(id as u64))) {
                    None => {
                        println!("gone");
                        return Ok(());
                    }
                    Some(s) if s["running"] != json!(true) => {
                        let code = c.call("session.status", json!({"id": id}))?["code"].clone();
                        println!("exited {code}");
                        return Ok(());
                    }
                    Some(s) if s["idle_ms"].as_u64().unwrap_or(0) >= idle_ms => {
                        println!("idle");
                        return Ok(());
                    }
                    _ => {}
                }
                if t0.elapsed() >= Duration::from_secs(timeout) {
                    eprintln!("timeout after {timeout}s");
                    std::process::exit(2);
                }
                std::thread::sleep(Duration::from_millis(500));
            }
        }
        Cmd::Search { text, max } => match c.call("agent.search", json!({"query": text, "max": max})) {
            Ok(r) => {
                let hits = r["hits"].as_array().cloned().unwrap_or_default();
                if hits.is_empty() {
                    println!("no hits");
                }
                for h in hits {
                    let snip = h["snippet"].as_str().unwrap_or_default().split_whitespace().collect::<Vec<_>>().join(" ");
                    println!(
                        "{}\t{}\t{}\t{}\t{}",
                        h["ts"].as_i64().unwrap_or(0),
                        h["agent"].as_str().unwrap_or("?"),
                        h["session"].as_str().unwrap_or("?"),
                        h["cwd"].as_str().unwrap_or(""),
                        snip
                    );
                }
            }
            Err(e) if e.to_string().contains("unknown method") => {
                bail!("this daemon has no agent.search; update and restart it")
            }
            Err(e) => return Err(e),
        },
        c2 @ (Cmd::Msg { .. }
        | Cmd::Inbox { .. }
        | Cmd::Ask { .. }
        | Cmd::Reply { .. }
        | Cmd::Done { .. }
        | Cmd::Escalate { .. }
        | Cmd::Note { .. }
        | Cmd::TaskFile
        | Cmd::Limit { .. }
        | Cmd::Thread { .. }
        | Cmd::Flow { .. }) => return super::orchcmd::run(c2, c),
        c2 @ Cmd::Computer { .. } => return super::computer::run(c2, c),
        _ => unreachable!(),
    }
    Ok(())
}

/// The first message for an agent: `--prompt TEXT`, `--prompt -` (stdin) or `--prompt-file PATH`.
fn resolve_prompt(prompt: Option<String>, file: Option<&Path>, stdin: &mut dyn std::io::Read) -> Result<Option<String>> {
    let text = match (prompt, file) {
        (Some(_), Some(_)) => bail!("use --prompt or --prompt-file, not both"),
        (Some(p), None) if p == "-" => {
            let mut t = String::new();
            stdin.read_to_string(&mut t).context("cannot read the prompt from stdin")?;
            t
        }
        (Some(p), None) => p,
        (None, Some(f)) => std::fs::read_to_string(f).with_context(|| format!("cannot read {}", f.display()))?,
        (None, None) => return Ok(None),
    };
    let text = text.trim_start_matches('\u{feff}').trim_end().to_string();
    if text.is_empty() {
        bail!("the prompt is empty");
    }
    Ok(Some(text))
}

/// `line` with the content of `file` as its last argument, read by the shell itself; the file is deleted once read.
/// Windows PowerShell 5 does not escape double quotes when it starts a program, so they are escaped here.
fn with_prompt_file(line: &str, file: &str, ps: bool) -> String {
    if ps {
        let f = ps_quote(file);
        format!(
            "$axp = [IO.File]::ReadAllText({f}, [Text.Encoding]::UTF8); Remove-Item -LiteralPath {f}; $axp = $axp -replace '(\\\\*)\"','$1$1\\\"' -replace '(\\\\+)$','$1$1'; {line} $axp"
        )
    } else {
        let f = sh_quote(file);
        format!("axp=$(cat {f}); rm -f {f}; {line} \"$axp\"")
    }
}

/// The flags that put `agent`'s interactive TUI on a first prompt, each with a leading space ("" when it needs none).
/// A session started with a prompt by an agent or a script is unattended, so the agent is also told to trust the
/// folder (gemini would stop at "Do you trust the files in this folder?").
fn prompt_flags(agent: &str) -> String {
    let Some(a) = axelstra_core::agents::get(agent) else { return String::new() };
    a.trust.iter().chain(a.interactive_prompt.unwrap_or(&[])).map(|f| format!(" {f}")).collect()
}

/// Starts a claude/codex session in `worktree` (visible in the app); returns its session id.
pub(crate) fn spawn_agent(
    c: &mut Client,
    worktree: &Path,
    agent: &str,
    prompt: Option<&str>,
    reap: Option<Value>,
    worker: bool,
) -> Result<u32> {
    // what the agent was asked, kept as the goal at the top of its task file
    let goal = prompt.unwrap_or_default().to_string();
    // any agent CLI on the PATH works; claude, codex and gemini get conversation resume
    if agent.is_empty() || !agent.chars().all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.')) {
        bail!("--agent must be the name of a command on the PATH (claude, codex, gemini...)");
    }
    let projects = c.call("project.list", json!({}))?;
    let (p, wt) = find_worktree(&projects, worktree)?;
    let dir = c.call("app.dir", json!({}))?["dir"].as_str().unwrap_or_default().to_string();
    let (program, mut args) = default_shell();
    let ps = program.to_ascii_lowercase().contains("powershell") || program.to_ascii_lowercase().contains("pwsh");
    let key = (agent == "claude").then(new_uuid);
    let hooks = format!("{dir}/hooks.json");
    let mut line = agent.to_string();
    if let Some(k) = &key {
        if Path::new(&hooks).exists() {
            line.push_str(&format!(" --settings {}", if ps { ps_quote(&hooks) } else { sh_quote(&hooks) }));
        }
        line.push_str(&format!(" --session-id {k}"));
    }
    // "short answers" setting: claude takes the rules as an appended system prompt, other agents get them in the prompt
    let rules = c.call("settings.get", json!({})).ok().and_then(|r| r["rules"].as_str().map(String::from));
    if let (Some(r), true) = (&rules, agent == "claude") {
        line.push_str(&format!(" --append-system-prompt {}", if ps { ps_quote(r) } else { sh_quote(r) }));
    }
    let prompt_owned = match (&rules, prompt) {
        (Some(r), Some(p)) if agent != "claude" => Some(format!(
            "{r}

{p}"
        )),
        (_, Some(p)) => Some(p.to_string()),
        _ => None,
    };
    let prompt = prompt_owned.as_deref();
    if let Some(pr) = prompt {
        // an ephemeral worker must know how to report: without it the daemon can only scrape its screen
        let full = if reap.is_some() && !pr.contains("axelstra done") { worker_prompt(pr) } else { pr.to_string() };
        // the text never goes through a command line: a file the shell reads, so <, >, &, quotes and newlines arrive as typed
        // an agent whose first prompt needs a flag (gemini: -i, which is explicit and also works before 0.65, where a bare prompt ran one-shot and exited) gets it here
        line.push_str(&prompt_flags(agent));
        let file = format!("{dir}/prompts/{}.txt", new_uuid());
        std::fs::create_dir_all(Path::new(&file).parent().context("bad data dir")?)?;
        std::fs::write(&file, &full).with_context(|| format!("cannot write {file}"))?;
        line = with_prompt_file(&line, &file, ps);
    }
    shell_run(&program, &mut args, ps, &line, reap.is_none());
    let mut meta = json!({"project": p["id"], "worktree": wt, "kind": "agent", "agent": agent});
    let mut env = json!({});
    if let Some(k) = &key {
        meta["resume"] = json!(k);
        meta["key"] = json!(k);
        env = json!({"AXELSTRA_KEY": k, "AXELSTRA_DIR": dir});
    }
    let r = c.call(
        "session.spawn",
        with_reap(
            json!({"program": program, "args": args, "cwd": wt, "label": agent, "meta": meta, "env": env, "goal": goal, "cols": 120, "rows": 30}),
            reap,
            worker,
        ),
    )?;
    Ok(r["id"].as_u64().unwrap_or(0) as u32)
}

fn print_ls(v: &Value) {
    let rows: Vec<[String; 4]> = v
        .as_array()
        .map(|a| a.as_slice())
        .unwrap_or_default()
        .iter()
        .map(|s| {
            let state = if s["running"] != json!(true) {
                "done"
            } else if s["idle_ms"].as_u64().unwrap_or(u64::MAX) < 3000 {
                "working"
            } else {
                "idle"
            };
            [
                s["id"].to_string(),
                s["label"].as_str().unwrap_or("-").to_string(),
                state.to_string(),
                s["meta"]["worktree"].as_str().unwrap_or("-").to_string(),
            ]
        })
        .collect();
    let mut w = [0usize; 3];
    for r in &rows {
        for i in 0..3 {
            w[i] = w[i].max(r[i].chars().count());
        }
    }
    for r in &rows {
        println!("{:<w0$}  {:<w1$}  {:<w2$}  {}", r[0], r[1], r[2], r[3], w0 = w[0], w1 = w[1], w2 = w[2]);
    }
}

pub(crate) fn norm(p: &str) -> String {
    p.replace('\\', "/").trim_end_matches('/').to_ascii_lowercase()
}

fn abs_norm(p: &Path) -> String {
    let a = std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf());
    norm(a.to_string_lossy().trim_start_matches(r"\\?\"))
}

/// Project whose id, name or path equals `r`.
pub(crate) fn find_project_by_ref<'a>(projects: &'a Value, r: &str) -> Option<&'a Value> {
    let want = abs_norm(Path::new(r));
    projects
        .as_array()?
        .iter()
        .find(|p| p["id"].as_str() == Some(r) || p["name"].as_str() == Some(r) || norm(p["path"].as_str().unwrap_or_default()) == want)
}

/// (project, worktree path as the daemon knows it) for a worktree directory.
pub(crate) fn find_worktree<'a>(projects: &'a Value, path: &Path) -> Result<(&'a Value, String)> {
    let want = abs_norm(path);
    for p in projects.as_array().map(|a| a.as_slice()).unwrap_or_default() {
        for w in p["worktrees"].as_array().map(|a| a.as_slice()).unwrap_or_default() {
            let wp = w["path"].as_str().unwrap_or_default();
            if norm(wp) == want {
                return Ok((p, wp.to_string()));
            }
        }
    }
    bail!("{} is not a worktree of a registered project (see `axelstra projects`)", path.display())
}

/// Same default as the app.
fn default_shell() -> (String, Vec<String>) {
    if cfg!(windows) {
        ("powershell".into(), vec!["-NoLogo".into()])
    } else {
        let sh = std::env::var("SHELL").unwrap_or_else(|_| if cfg!(target_os = "macos") { "/bin/zsh".into() } else { "/bin/bash".into() });
        (sh, vec!["-l".into()])
    }
}

/// Makes the shell run `line` and stay open afterwards.
fn shell_run(program: &str, args: &mut Vec<String>, ps: bool, line: &str, keep_open: bool) {
    if ps {
        if keep_open {
            args.push("-NoExit".into());
        }
        args.extend(["-Command".into(), line.into()]);
    } else if keep_open {
        args.extend(["-c".into(), format!("{line}; exec {} -l", sh_quote(program))]);
    } else {
        args.extend(["-c".into(), line.into()]);
    }
}

/// What an ephemeral worker starts with: its task, how to report, and the rule against waiting silently for the human.
fn worker_prompt(task: &str) -> String {
    format!(
        "{task}\n\nWhen finished, run `axelstra done \"<summary: what you did, what is left>\"` (add `--status failed` or `--status blocked` when it applies). {}",
        axelstra_core::waiting::NEVER_WAIT_RULE
    )
}

/// `--report-to` resolution: Some(None) means "this session", from $AXELSTRA_SESSION.
pub(crate) fn reap_for(
    report_to: Option<Option<u32>>,
    when: Option<&str>,
    default_when: &str,
    idle_secs: Option<u64>,
    lines: Option<u32>,
    env_session: Option<&str>,
) -> Result<Option<Value>> {
    let Some(rt) = report_to else { return Ok(None) };
    let parent = match rt {
        Some(id) => id,
        None => env_session
            .and_then(|s| s.trim().parse().ok())
            .context("--report-to needs an id: $AXELSTRA_SESSION is not set (not inside a session)")?,
    };
    let mode = when.unwrap_or(default_when);
    if mode != "exit" && mode != "idle" {
        bail!("--when must be exit or idle");
    }
    let mut r = json!({"parent": parent, "mode": mode});
    if let Some(s) = idle_secs {
        r["idle_secs"] = json!(s);
    }
    if let Some(l) = lines {
        r["lines"] = json!(l);
    }
    Ok(Some(r))
}

/// Adds the worker bookkeeping to a spawn request: the reaper, the parent session (the one running this
/// CLI, if any) and whether the session counts against the worker limit.
pub(crate) fn with_reap(mut params: Value, reap: Option<Value>, worker: bool) -> Value {
    let env_parent = std::env::var("AXELSTRA_SESSION").ok().and_then(|s| s.trim().parse::<u32>().ok());
    if let Some(r) = reap {
        params["parent"] = r["parent"].clone();
        params["reap"] = r;
    } else if let Some(p) = env_parent {
        params["parent"] = json!(p);
    }
    if worker {
        params["worker"] = json!(true);
    }
    params
}

/// Program and args that run one command line and exit (no interactive shell left behind).
pub(crate) fn one_shot_line(line: &str) -> (String, Vec<String>) {
    if cfg!(windows) {
        ("cmd".into(), vec!["/d".into(), "/c".into(), line.into()])
    } else {
        ("sh".into(), vec!["-c".into(), line.into()])
    }
}

fn one_shot_args(command: &[String]) -> (String, Vec<String>) {
    if cfg!(windows) {
        let mut a = vec!["/d".into(), "/c".into()];
        a.extend(command.iter().cloned());
        ("cmd".into(), a)
    } else {
        (command[0].clone(), command[1..].to_vec())
    }
}

fn new_uuid() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut b = [0u8; 16];
    for (i, ch) in b.chunks_mut(8).enumerate() {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos()));
        h.write_u32(std::process::id());
        h.write_usize(i);
        ch.copy_from_slice(&h.finish().to_le_bytes());
    }
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let x: String = b.iter().map(|v| format!("{v:02x}")).collect();
    format!("{}-{}-{}-{}-{}", &x[0..8], &x[8..12], &x[12..16], &x[16..20], &x[20..32])
}

/// Escapes: \e \r \n \xHH, `\\` and any other `\c` is `c`.
fn unescape(data: &str) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let b = data.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == 0x5c && i + 1 < b.len() {
            match b[i + 1] {
                b'e' => out.push(0x1b),
                b'r' => out.push(0x0d),
                b'n' => out.push(0x0a),
                b'x' if i + 3 < b.len() => {
                    out.push(u8::from_str_radix(data.get(i + 2..i + 4).context("bad \\x escape")?, 16)?);
                    i += 2;
                }
                o => out.push(o),
            }
            i += 2;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    Ok(out)
}

/// Removes CSI/OSC/other escape sequences and resolves bare `\r` as "overwrite the line".
pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(ch) = it.next() {
        match ch {
            '\x1b' => match it.next() {
                Some('[') => {
                    for n in it.by_ref() {
                        if ('\u{40}'..='\u{7e}').contains(&n) {
                            break;
                        }
                    }
                }
                Some(']') | Some('P') | Some('_') | Some('^') => {
                    while let Some(n) = it.next() {
                        if n == '\x07' {
                            break;
                        }
                        if n == '\x1b' {
                            it.next(); // ESC \
                            break;
                        }
                    }
                }
                Some('(') | Some(')') | Some('#') => {
                    it.next();
                }
                _ => {}
            },
            '\r' => {
                if it.peek() != Some(&'\n') {
                    let start = out.rfind('\n').map_or(0, |i| i + 1);
                    out.truncate(start);
                }
            }
            '\x07' | '\x08' | '\0' => {}
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_prompt_says_how_to_report_and_never_to_wait_silently() {
        let p = worker_prompt("do the thing");
        assert!(p.starts_with("do the thing"));
        assert!(p.contains("axelstra done"));
        assert!(p.contains("Never wait silently for the human"));
        assert!(p.contains("axelstra ask $AXELSTRA_PARENT") && p.contains("axelstra escalate"));
    }

    #[test]
    fn strips_sequences() {
        assert_eq!(strip_ansi("\x1b[31mred\x1b[0m \x1b]0;title\x07ok\r\nnext\rover"), "red ok\nover");
    }

    #[test]
    fn reap_resolution() {
        assert!(reap_for(None, None, "exit", None, None, Some("3")).unwrap().is_none());
        assert_eq!(
            reap_for(Some(Some(7)), None, "exit", None, Some(10), None).unwrap().unwrap(),
            json!({"parent": 7, "mode": "exit", "lines": 10})
        );
        assert_eq!(
            reap_for(Some(None), Some("idle"), "exit", Some(5), None, Some("3")).unwrap().unwrap(),
            json!({"parent": 3, "mode": "idle", "idle_secs": 5})
        );
        assert!(reap_for(Some(None), None, "exit", None, None, None).is_err());
        assert!(reap_for(Some(Some(1)), Some("x"), "exit", None, None, None).is_err());
    }

    #[test]
    fn uuid_shape() {
        let u = new_uuid();
        assert_eq!(u.len(), 36);
        assert_ne!(u, new_uuid());
    }

    const NASTY: &str =
        "corrige <login> & testa > saída | \"citado\" 'simples' \\\"barra\\ $HOME `tick` %PATH% ^ !\nsegunda linha\n\n  recuo\\";

    #[test]
    fn gemini_needs_the_interactive_prompt_flag_the_others_do_not() {
        assert_eq!(prompt_flags("gemini"), " --skip-trust -i");
        assert_eq!((prompt_flags("claude"), prompt_flags("codex"), prompt_flags("other")), (String::new(), String::new(), String::new()));
    }

    #[test]
    fn prompt_comes_from_text_stdin_or_file() {
        let mut empty = std::io::empty();
        assert_eq!(resolve_prompt(Some("a<b".into()), None, &mut empty).unwrap().as_deref(), Some("a<b"));
        assert_eq!(resolve_prompt(None, None, &mut empty).unwrap(), None);
        let mut input = std::io::Cursor::new(NASTY.as_bytes().to_vec());
        assert_eq!(resolve_prompt(Some("-".into()), None, &mut input).unwrap().unwrap(), NASTY.trim_end());
        let f = std::env::temp_dir().join(format!("axl-prompt-{}.txt", std::process::id()));
        std::fs::write(&f, format!("\u{feff}{NASTY}\n")).unwrap();
        assert_eq!(resolve_prompt(None, Some(&f), &mut empty).unwrap().unwrap(), NASTY.trim_end());
        assert!(resolve_prompt(Some("x".into()), Some(&f), &mut empty).is_err());
        std::fs::write(&f, " \n").unwrap();
        assert!(resolve_prompt(None, Some(&f), &mut empty).is_err());
        let _ = std::fs::remove_file(&f);
    }

    #[test]
    fn posix_line_reads_the_prompt_from_the_file() {
        let l = with_prompt_file("claude --x", "/tmp/it's.txt", false);
        assert_eq!(l, r#"axp=$(cat '/tmp/it'\''s.txt'); rm -f '/tmp/it'\''s.txt'; claude --x "$axp""#);
    }

    #[cfg(unix)]
    #[test]
    fn posix_shell_passes_the_prompt_unchanged() {
        let d = std::env::temp_dir().join(format!("axl-posix-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let (input, output) = (d.join("in.txt"), d.join("out.txt"));
        std::fs::write(&input, NASTY).unwrap();
        let line = with_prompt_file("printf %s", &input.to_string_lossy(), false);
        let st =
            std::process::Command::new("sh").args(["-c", &format!("{line} > {}", sh_quote(&output.to_string_lossy()))]).status().unwrap();
        assert!(st.success());
        assert_eq!(std::fs::read_to_string(&output).unwrap(), NASTY);
        assert!(!input.exists(), "prompt file is removed once read");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// The real path on Windows: powershell -Command LINE, the program receiving the text as one argument.
    /// A second powershell stands in for the agent: it writes the argument it received to a file.
    #[cfg(windows)]
    #[test]
    fn windows_line_delivers_the_prompt_unchanged() {
        let d = std::env::temp_dir().join(format!("axl-ps-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let (script, input, output) = (d.join("echo.ps1"), d.join("in.txt"), d.join("out.txt"));
        std::fs::write(
            &script,
            format!(
                "if ($args.Count -ne 1) {{ [IO.File]::WriteAllText('{}', 'ARGS=' + $args.Count); exit 1 }}\r\n[IO.File]::WriteAllText('{}', $args[0], (New-Object Text.UTF8Encoding $false))",
                output.display(),
                output.display()
            ),
        )
        .unwrap();
        for prompt in [NASTY, "simples", "termina em barra\\", "só \"aspas\""] {
            let _ = std::fs::remove_file(&output);
            std::fs::write(&input, prompt).unwrap();
            let agent = format!("powershell -NoProfile -NonInteractive -File {}", ps_quote(&script.to_string_lossy()));
            let line = with_prompt_file(&agent, &input.to_string_lossy(), true);
            let st = std::process::Command::new("powershell").args(["-NoProfile", "-NonInteractive", "-Command", &line]).status().unwrap();
            assert!(st.success(), "{prompt:?}: {:?}", std::fs::read_to_string(&output));
            assert_eq!(std::fs::read_to_string(&output).unwrap(), prompt, "{prompt:?}");
            assert!(!input.exists(), "prompt file is removed once read");
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn unescapes() {
        assert_eq!(unescape("a\\r\\x1b").unwrap(), vec![b'a', 13, 0x1b]);
    }
}
