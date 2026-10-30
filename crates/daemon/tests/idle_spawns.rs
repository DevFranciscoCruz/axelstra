//! A desktop near idle must not start a storm of child processes. Two projects with four worktrees each, agents that do
//! their bookkeeping (hook hints, inbox reads, quiet messages: calls that bump the kept answers' generation), one agent
//! editing a file in the selected worktree every 5 s, the app's watch on that worktree and its lists read again every
//! 2 s (more often than the real app, which reads on events). An edit in one worktree may cost the git of that one
//! worktree (its status, its uncommitted files, its change list), never a new look at every worktree: before the fixes
//! each edit re-ran `git diff` and `git ls-files` in all of them (`dispatch.overlap_files`) and the janitor swept
//! again. Same pattern as `tools/idle-spawns.mjs`, short. Spawns are counted by `core::proc::hidden` (`debug.load`);
//! the sites (`mem.status` breakdown.own.sites) are printed.
mod common;
use axelstra_daemon::client::Client;
use axelstra_daemon::server;
use serde_json::json;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

/// Warm-up: the first looks (and what they set off: janitor, PR watcher, watches) are allowed to run git.
const WARMUP: Duration = Duration::from_secs(15);
/// The measured window.
const WINDOW: Duration = Duration::from_secs(45);
/// Time between two edits in the selected worktree.
const EDIT_EVERY: Duration = Duration::from_secs(5);
/// Children one edit may cost: the git of the edited worktree only (its change list 3, status, overlaps, overview:
/// about 5 once each child is counted once). Before the fixes one edit cost about 13 (8 edits, 104 children).
const PER_EDIT: u64 = 6;
/// Children the window may start besides the edits: one janitor sweep, the age floor of a checkout nobody watches.
const SLACK: u64 = 12;

fn git(cwd: &Path, args: &[&str]) {
    let out = Command::new("git").current_dir(cwd).args(args).output().unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

/// A long-lived shell that prints its own agent credential: its id and a client that uses it.
fn agent_session(app: &mut Client, pipe: &str, token: &str, cwd: &str) -> (u32, Client) {
    let (prog, args) = if cfg!(windows) {
        ("cmd", json!(["/q", "/k", "echo TOK[%AXELSTRA_AGENT_TOKEN%]"]))
    } else {
        ("sh", json!(["-c", "echo TOK[$AXELSTRA_AGENT_TOKEN]; exec cat"]))
    };
    let id = app
        .call("session.spawn", json!({"program": prog, "args": args, "cwd": cwd, "meta": {"kind": "agent", "worktree": cwd}}))
        .unwrap()["id"]
        .as_u64()
        .unwrap() as u32;
    let t = Instant::now();
    let agent_token = loop {
        let lines = app.call("session.screen", json!({"id": id, "lines": 100})).unwrap()["lines"].to_string();
        if let Some(a) = lines.find("TOK[") {
            let rest = &lines[a + 4..];
            if let Some(b) = rest.find(']') {
                if b >= 32 {
                    break rest[..b].to_string();
                }
            }
        }
        assert!(t.elapsed() < Duration::from_secs(15), "no token printed: {lines}");
        std::thread::sleep(Duration::from_millis(200));
    };
    (id, Client::connect_agent(pipe, token, &agent_token).unwrap())
}

/// Children started so far per "site  command" (the busiest rows of `mem.status`).
fn sites(c: &mut Client) -> HashMap<String, u64> {
    let st = c.call("mem.status", json!({})).unwrap();
    st["breakdown"]["own"]["sites"]
        .as_array()
        .map(|rows| {
            rows.iter()
                .map(|r| {
                    (
                        format!("{}  {}", r["site"].as_str().unwrap_or("?"), r["command"].as_str().unwrap_or("?")),
                        r["total"].as_u64().unwrap_or(0),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

fn spawns(c: &mut Client) -> u64 {
    c.call("debug.load", json!({})).unwrap()["spawns"].as_u64().unwrap()
}

#[test]
fn a_desktop_near_idle_starts_few_child_processes() {
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(240));
        eprintln!("watchdog: test stuck");
        std::process::exit(1);
    });
    let root: PathBuf = std::env::temp_dir().join(format!("axelstra-idlespawns-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    // git identity for the repos and the daemon's own commits, never the developer's gitconfig
    let gitconfig = root.join("gitconfig");
    std::fs::write(&gitconfig, "[user]\n\temail = t@t\n\tname = t\n").unwrap();
    std::env::set_var("GIT_CONFIG_GLOBAL", &gitconfig);
    std::env::set_var("GIT_TERMINAL_PROMPT", "0");

    let pipe = format!("{}-idlespawns-{}", common::base_pipe(), std::process::id());
    let token = "s".repeat(32);
    let (p, t, s) = (pipe.clone(), token.clone(), root.join("projects.json"));
    std::thread::spawn(move || {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(4)
            .enable_all()
            .build()
            .unwrap()
            .block_on(server::serve_with_store(p, t, s))
            .unwrap();
    });
    common::wait_ready(&pipe, &token);
    let mut app = Client::connect(&pipe, &token).unwrap();

    // two projects, four worktrees each (three auto as `dispatch new` makes them, one by hand; one with work not yet in
    // main, one dirty)
    let mut paths: Vec<String> = vec![];
    let mut wts: Vec<String> = vec![];
    for i in 0..2 {
        let repo = root.join(format!("p{i}"));
        std::fs::create_dir_all(repo.join("src")).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        std::fs::write(repo.join("a.txt"), "x\n").unwrap();
        std::fs::write(repo.join("src").join("lib.txt"), "lib\n").unwrap();
        git(&repo, &["add", "a.txt", "src/lib.txt"]);
        git(&repo, &["commit", "-qm", "init"]);
        let id = app.call("project.add", json!({"path": &repo})).unwrap()["id"].as_str().unwrap().to_string();
        paths.push(repo.to_string_lossy().to_string());
        for k in 0..4 {
            let mark = if k == 3 { json!({"manual": true}) } else { json!({"auto": true}) };
            let mut params = json!({"project": id, "name": format!("w{k}")});
            params.as_object_mut().unwrap().extend(mark.as_object().unwrap().clone());
            let wt = app.call("worktree.create", params).unwrap()["path"].as_str().unwrap().to_string();
            let dir = Path::new(&wt);
            if k == 1 {
                std::fs::write(dir.join("w.txt"), "work\n").unwrap();
                git(dir, &["add", "w.txt"]);
                git(dir, &["commit", "-qm", "work"]);
            }
            if k == 2 {
                std::fs::write(dir.join("a.txt"), "dirty\n").unwrap();
            }
            paths.push(wt.clone());
            wts.push(wt);
        }
    }

    // agents in the first two worktrees of each project, a finished command (its exit wakes the janitor)
    let mut agents: Vec<(u32, Client)> = vec![];
    for w in [&wts[0], &wts[1], &wts[4], &wts[5]] {
        agents.push(agent_session(&mut app, &pipe, &token, w));
    }
    let once = if cfg!(windows) {
        json!({"program": "cmd", "args": ["/c", "exit 0"]})
    } else {
        json!({"program": "sh", "args": ["-c", "exit 0"]})
    };
    app.call("session.spawn", once).unwrap();

    // the app: a watch on the selected worktree, its lists
    let selected = wts[0].clone();
    let mut watch = Client::connect(&pipe, &token).unwrap();
    watch.call("watch.subscribe", json!({"worktree": &selected})).unwrap();
    std::thread::spawn(move || while watch.read().is_ok() {});
    let lists = |app: &mut Client| {
        for (m, p) in [
            ("session.list", json!({})),
            ("project.list", json!({})),
            ("worktree.overview", json!({})),
            ("worktree.status", json!({"paths": &paths})),
            ("dispatch.overlap_files", json!({})),
            ("git.changes", json!({"worktree": &selected})),
        ] {
            app.call(m, p).unwrap_or_else(|e| panic!("{m}: {e:#}"));
        }
    };
    let main_id = agents[0].0;
    let mut turn = 0u64;
    // the agents' bookkeeping, one call every 250 ms, an edit every `EDIT_EVERY` and the app's lists every 2 s, for
    // `d`; returns the number of edits
    let mut idle = |app: &mut Client, agents: &mut Vec<(u32, Client)>, d: Duration| {
        let end = Instant::now() + d;
        let mut next_lists = Instant::now();
        let mut next_edit = Instant::now() + EDIT_EVERY;
        let mut edits = 0u64;
        while Instant::now() < end {
            if Instant::now() >= next_edit {
                let f = Path::new(&selected).join("notes.txt");
                let old = std::fs::read_to_string(&f).unwrap_or_default();
                std::fs::write(&f, format!("{old}turn {turn}\n")).unwrap();
                edits += 1;
                next_edit += EDIT_EVERY;
            }
            if Instant::now() >= next_lists {
                lists(app);
                next_lists += Duration::from_secs(2);
            }
            turn += 1;
            let n = agents.len() as u64;
            let (id, c) = &mut agents[(turn % n) as usize];
            let id = *id;
            match (turn / n) % 4 {
                0 => c.call("session.hint", json!({"id": id, "state": "working"})),
                1 => c.call("msg.inbox", json!({"session": id, "mark": true})),
                2 if id != main_id => c.call(
                    "msg.send",
                    json!({"from": id, "to": main_id, "text": format!("progress {turn}"), "kind": "message", "quiet": true}),
                ),
                _ => c.call("session.hint", json!({"id": id, "state": "idle"})),
            }
            .unwrap();
            std::thread::sleep(Duration::from_millis(250));
        }
        edits
    };

    idle(&mut app, &mut agents, WARMUP);
    let (before, sites_before) = (spawns(&mut app), sites(&mut app));
    let t0 = Instant::now();
    let edits = idle(&mut app, &mut agents, WINDOW);
    let (after, sites_after) = (spawns(&mut app), sites(&mut app));
    let n = after - before;

    let mut rows: Vec<(u64, &String)> =
        sites_after.iter().map(|(k, v)| (v - sites_before.get(k).copied().unwrap_or(0), k)).filter(|(d, _)| *d > 0).collect();
    rows.sort_by_key(|r| std::cmp::Reverse(r.0));
    let table: String = rows.iter().map(|(d, k)| format!("\n  {d:>5}  {k}")).collect();
    let mins = t0.elapsed().as_secs_f64() / 60.0;
    let limit = edits * PER_EDIT + SLACK;
    eprintln!(
        "idle_spawns: {n} children in {:.0} s ({:.1}/min), {edits} edits, limit {limit}; by site:{table}",
        t0.elapsed().as_secs_f64(),
        n as f64 / mins
    );
    let _ = std::fs::remove_dir_all(&root);
    assert!(n <= limit, "{n} children in the window for {edits} edits (limit {limit}); by site:{table}");
}
