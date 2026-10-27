//! `axelstra wait` and `axelstra ask` are woken by the daemon's change events (`session.changed`, `session.exited`),
//! not by a poll: a real daemon, the real CLI binary, and the events a second connection receives.

use axelstra_daemon::client::Client;
use axelstra_daemon::proto;
#[cfg(not(unix))]
use axelstra_daemon::server;
use serde_json::{json, Value};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

struct Env {
    root: std::path::PathBuf,
    #[cfg(unix)]
    _daemon: std::process::Child,
}

/// One daemon for the whole file, on a pipe name of its own and a throwaway data dir. The environment is set before any
/// thread reads it (every test goes through here first) and is inherited by the bridges the tests start.
fn env() -> &'static Env {
    static ENV: OnceLock<Env> = OnceLock::new();
    ENV.get_or_init(|| {
        let root = std::env::temp_dir().join(format!("axelstra-waitev-{}", std::process::id()));
        let data = root.join("data");
        std::fs::create_dir_all(&data).unwrap();
        // not `data` itself: on Unix the default dir would then be the override and the serve guard refuses it
        let home = root.join("home");
        std::fs::create_dir_all(&home).unwrap();
        std::env::set_var("LOCALAPPDATA", &home);
        std::env::set_var("XDG_DATA_HOME", &home);
        std::env::set_var("HOME", &home);
        // AXELSTRA_DATA_DIR wins over the three above, so it must point at this throwaway dir too
        std::env::set_var("AXELSTRA_DATA_DIR", data.join("axelstra"));
        std::env::set_var("AXELSTRA_TEST_PIPE", format!("we{}", std::process::id()));
        std::env::set_var("USERNAME", format!("axwe{}", std::process::id()));
        std::env::remove_var("AXELSTRA_AGENT_TOKEN");
        let token = "w".repeat(32);
        std::fs::create_dir_all(proto::try_data_dir().unwrap()).unwrap();
        std::fs::write(proto::token_path(), &token).unwrap();
        // Unix: the daemon is a process of its own. Served in this process, the bridge (our child) would descend from
        // the daemon and be taken for an orphaned agent; a real bridge is never below the daemon.
        #[cfg(unix)]
        let daemon = {
            #[cfg(target_os = "linux")]
            use std::os::unix::process::CommandExt;
            let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_axelstra"));
            cmd.arg("daemon").env("AXELSTRA_IDLE_EXIT_SECS", "120").stdin(std::process::Stdio::null());
            // SAFETY: only prctl between fork and exec; the daemon must not outlive the test run. The parent-death
            // signal is tied to the thread that forked, so a thread of its own does the fork and then stays parked.
            #[cfg(target_os = "linux")]
            unsafe {
                cmd.pre_exec(|| {
                    libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL as libc::c_ulong, 0, 0, 0);
                    Ok(())
                });
            }
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                tx.send(cmd.spawn().unwrap()).unwrap();
                loop {
                    std::thread::park();
                }
            });
            let daemon = rx.recv().unwrap();
            let end = Instant::now() + Duration::from_secs(10);
            while std::os::unix::net::UnixStream::connect(proto::pipe_name()).is_err() && Instant::now() < end {
                std::thread::sleep(Duration::from_millis(50));
            }
            daemon
        };
        #[cfg(not(unix))]
        {
            let (pipe, store) = (proto::pipe_name(), root.join("projects.json"));
            std::thread::spawn(move || {
                tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(2)
                    .enable_all()
                    .build()
                    .unwrap()
                    .block_on(server::serve_with_store(pipe, token, store))
                    .unwrap();
            });
            std::thread::sleep(Duration::from_millis(500));
        }
        Env {
            root,
            #[cfg(unix)]
            _daemon: daemon,
        }
    })
}

fn client() -> Client {
    env();
    Client::connect_cli(&proto::pipe_name(), &proto::load_or_create_token().unwrap()).unwrap()
}

fn shell() -> (&'static str, Vec<&'static str>) {
    if cfg!(windows) {
        ("cmd", vec!["/q"])
    } else {
        ("sh", vec![])
    }
}

fn spawn(c: &mut Client, label: &str) -> u64 {
    let (prog, args) = shell();
    c.call("session.spawn", json!({"program": prog, "args": args, "label": label, "meta": {"kind": "agent"}})).unwrap()["id"]
        .as_u64()
        .unwrap()
}

fn axelstra(session: Option<u64>) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_axelstra"));
    c.env_remove("AXELSTRA_AGENT_TOKEN").env_remove("AXELSTRA_HOST").stdout(Stdio::piped()).stderr(Stdio::piped());
    if let Some(s) = session {
        c.env("AXELSTRA_SESSION", s.to_string());
    }
    c
}

/// Waits for a child with a watchdog so a command that never wakes fails the test instead of hanging it.
fn finish(child: std::process::Child, secs: u64) -> (bool, String, String, Duration) {
    let t = Instant::now();
    let pid = child.id();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(child.wait_with_output());
    });
    match rx.recv_timeout(Duration::from_secs(secs)) {
        Ok(Ok(o)) => (
            o.status.success(),
            String::from_utf8_lossy(&o.stdout).into_owned(),
            String::from_utf8_lossy(&o.stderr).into_owned(),
            t.elapsed(),
        ),
        _ => {
            let _ = Command::new(if cfg!(windows) { "taskkill" } else { "kill" })
                .args(if cfg!(windows) {
                    vec!["/F".to_string(), "/PID".into(), pid.to_string()]
                } else {
                    vec!["-9".into(), pid.to_string()]
                })
                .status();
            panic!("the command did not finish in {secs}s");
        }
    }
}

#[test]
fn wait_returns_when_the_session_exits_with_no_poll_period_to_wait_for() {
    let mut c = client();
    let id = spawn(&mut c, "waited");
    let child = axelstra(None).args(["wait", &id.to_string(), "--idle-secs", "3600", "--timeout", "120"]).spawn().unwrap();
    // let the command subscribe and take its first look
    std::thread::sleep(Duration::from_millis(1500));
    c.call("session.kill", json!({"id": id})).unwrap();
    let (_, out, err, took) = finish(child, 60);
    assert!(out.contains("exited") || out.contains("gone"), "stdout: {out} stderr: {err}");
    assert!(took < Duration::from_secs(30), "took {took:?}");
}

#[test]
fn wait_notices_idle_at_the_threshold_without_any_event() {
    let mut c = client();
    let id = spawn(&mut c, "quiet");
    let child = axelstra(None).args(["wait", &id.to_string(), "--idle-secs", "2", "--timeout", "120"]).spawn().unwrap();
    let (_, out, err, took) = finish(child, 90);
    assert!(out.contains("idle"), "stdout: {out} stderr: {err}");
    assert!(took < Duration::from_secs(60), "took {took:?}");
    let _ = c.call("session.kill", json!({"id": id}));
}

#[test]
fn wait_times_out_with_exit_2_on_its_own_deadline() {
    let mut c = client();
    let id = spawn(&mut c, "busy");
    let child = axelstra(None).args(["wait", &id.to_string(), "--idle-secs", "3600", "--timeout", "2"]).spawn().unwrap();
    let mut child = child;
    let t = Instant::now();
    let status = {
        let end = Instant::now() + Duration::from_secs(60);
        loop {
            if let Some(s) = child.try_wait().unwrap() {
                break s;
            }
            assert!(Instant::now() < end, "wait did not time out");
            std::thread::sleep(Duration::from_millis(50));
        }
    };
    assert_eq!(status.code(), Some(2));
    assert!(t.elapsed() >= Duration::from_secs(1));
    let _ = c.call("session.kill", json!({"id": id}));
}

#[test]
fn ask_returns_the_moment_the_reply_arrives() {
    let mut c = client();
    let asker = spawn(&mut c, "asker");
    let answerer = spawn(&mut c, "answerer");
    let child = axelstra(Some(asker)).args(["ask", &answerer.to_string(), "which db?", "--timeout", "120"]).spawn().unwrap();
    // the question shows up in the answerer's inbox
    let end = Instant::now() + Duration::from_secs(30);
    let q = loop {
        let inbox: Value = c.call("msg.inbox", json!({"session": answerer, "all": true, "mark": false})).unwrap();
        if let Some(m) = inbox.as_array().and_then(|a| a.iter().find(|m| m["kind"] == "ask")) {
            break m["id"].as_u64().unwrap();
        }
        assert!(Instant::now() < end, "the question never arrived");
        std::thread::sleep(Duration::from_millis(50));
    };
    // let the asker settle into its wait, then answer
    std::thread::sleep(Duration::from_millis(1000));
    c.call("msg.send", json!({"from": answerer, "to": asker, "text": "postgres", "kind": "reply", "reply_to": q, "quiet": true})).unwrap();
    let (ok, out, err, took) = finish(child, 60);
    assert!(ok && out.contains("postgres"), "stdout: {out} stderr: {err}");
    assert!(took < Duration::from_secs(30), "took {took:?}");
    let _ = c.call("session.kill", json!({"id": asker}));
    let _ = c.call("session.kill", json!({"id": answerer}));
}

#[test]
fn ask_gives_up_at_its_deadline_when_nobody_answers() {
    let mut c = client();
    let asker = spawn(&mut c, "asker2");
    let other = spawn(&mut c, "silent");
    let child = axelstra(Some(asker)).args(["ask", &other.to_string(), "anyone?", "--timeout", "2"]).spawn().unwrap();
    let (ok, _, err, took) = finish(child, 60);
    assert!(!ok && err.contains("no answer after 2s"), "stderr: {err}");
    assert!(took < Duration::from_secs(30), "took {took:?}");
    let _ = c.call("session.kill", json!({"id": asker}));
    let _ = c.call("session.kill", json!({"id": other}));
}

#[test]
fn flow_runs_chained_tasks_on_events_and_ends_when_the_last_exits() {
    let mut c = client();
    let repo = env().root.join("flowrepo");
    std::fs::create_dir_all(&repo).unwrap();
    for args in [vec!["init", "-q", "-b", "main"], vec!["config", "user.email", "t@t"], vec!["config", "user.name", "t"]] {
        assert!(Command::new("git").current_dir(&repo).args(args).status().unwrap().success());
    }
    std::fs::write(repo.join("f.txt"), "x").unwrap();
    assert!(Command::new("git").current_dir(&repo).args(["add", "f.txt"]).status().unwrap().success());
    assert!(Command::new("git").current_dir(&repo).args(["commit", "-qm", "init"]).status().unwrap().success());
    let repo = std::fs::canonicalize(&repo).unwrap();
    c.call("project.add", json!({"path": repo.to_string_lossy()})).unwrap();
    let flow = env().root.join("flow.json");
    let wt = repo.to_string_lossy().to_string();
    std::fs::write(
        &flow,
        json!({"tasks": [
            {"id": "one", "worktree": wt, "cmd": "echo first-done"},
            {"id": "two", "worktree": wt, "cmd": "echo second-done", "after": ["one"]},
        ]})
        .to_string(),
    )
    .unwrap();
    let child = axelstra(None).args(["flow", &flow.to_string_lossy(), "--timeout", "3600"]).spawn().unwrap();
    let (ok, out, err, took) = finish(child, 90);
    assert!(ok, "stdout: {out} stderr: {err}");
    assert!(out.contains("one: ok") && out.contains("two: ok"), "stdout: {out}");
    assert!(out.contains("finished: 2 ok, 0 failed"), "stdout: {out}");
    assert!(took < Duration::from_secs(60), "took {took:?}");
}
