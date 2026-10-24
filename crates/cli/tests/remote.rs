//! `axelstra bridge` against a real daemon, reached the way `--host` reaches a remote one: as a child process whose
//! stdin/stdout carry the frames. The spec is the test-only `Exec` one that runs the bridge directly, so neither ssh nor
//! WSL is needed. The "remote machine" is this one, with its own data dir (token file) and pipe name.

use axelstra_daemon::client::Client;
use axelstra_daemon::proto;
use axelstra_daemon::remote::RemoteSpec;
#[cfg(not(unix))]
use axelstra_daemon::server;
use serde_json::json;
use std::path::Path;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

struct Env {
    root: std::path::PathBuf,
}

/// One daemon for the whole file, on a pipe name of its own and a throwaway data dir. The environment is set before any
/// thread reads it (every test goes through here first) and is inherited by the bridges the tests start.
fn env() -> &'static Env {
    static ENV: OnceLock<Env> = OnceLock::new();
    ENV.get_or_init(|| {
        let root = std::env::temp_dir().join(format!("axelstra-bridge-{}", std::process::id()));
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
        std::env::set_var("AXELSTRA_TEST_PIPE", format!("br{}", std::process::id()));
        std::env::set_var("USERNAME", format!("axbr{}", std::process::id()));
        std::env::remove_var("AXELSTRA_AGENT_TOKEN");
        let token = "b".repeat(32);
        std::fs::create_dir_all(proto::try_data_dir().unwrap()).unwrap();
        std::fs::write(proto::token_path(), &token).unwrap();
        // Unix: the daemon is a process of its own. Served in this process, the bridge (our child) would descend from
        // the daemon and be taken for an orphaned agent; a real bridge is never below the daemon.
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_axelstra"));
            cmd.arg("daemon").env("AXELSTRA_IDLE_EXIT_SECS", "120").stdin(std::process::Stdio::null());
            // SAFETY: only prctl between fork and exec; the daemon must not outlive the test run
            #[cfg(target_os = "linux")]
            unsafe {
                cmd.pre_exec(|| {
                    libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL as libc::c_ulong, 0, 0, 0);
                    Ok(())
                });
            }
            cmd.spawn().unwrap();
            let end = Instant::now() + Duration::from_secs(10);
            while !Path::new(&proto::pipe_name()).exists() && Instant::now() < end {
                std::thread::sleep(Duration::from_millis(50));
            }
        }
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
        Env { root }
    })
}

fn bridge_spec(extra_env: &[(&str, &Path)]) -> RemoteSpec {
    RemoteSpec::Exec {
        program: env!("CARGO_BIN_EXE_axelstra").into(),
        args: vec!["bridge".into()],
        env: extra_env.iter().map(|(k, v)| (k.to_string(), v.to_string_lossy().into_owned())).collect(),
    }
}

/// A machine of its own: another data dir with `token` in it (or not) and a pipe nobody serves.
fn elsewhere(dir: &Path, token: Option<&str>) -> RemoteSpec {
    std::fs::create_dir_all(dir.join("axelstra")).unwrap();
    if let Some(t) = token {
        std::fs::write(dir.join("axelstra").join("token"), t).unwrap();
    }
    let RemoteSpec::Exec { program, args, mut env } = bridge_spec(&[("LOCALAPPDATA", dir), ("XDG_DATA_HOME", dir), ("HOME", dir)]) else {
        unreachable!()
    };
    env.push(("AXELSTRA_DATA_DIR".into(), dir.join("axelstra").to_string_lossy().into_owned()));
    env.push(("AXELSTRA_TEST_PIPE".into(), format!("nodaemon{}", std::process::id())));
    // Windows: a missing pipe falls back to the pre-SID name built from USERNAME, which may be a real daemon
    env.push(("USERNAME".into(), format!("axnodaemon{}", std::process::id())));
    RemoteSpec::Exec { program, args, env }
}

fn alive(pid: u32) -> bool {
    if cfg!(windows) {
        let out = std::process::Command::new("tasklist").args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"]).output().unwrap();
        String::from_utf8_lossy(&out.stdout).contains(&format!("\"{pid}\""))
    } else {
        std::process::Command::new("kill").args(["-0", &pid.to_string()]).output().unwrap().status.success()
    }
}

fn wait_dead(pid: u32) -> bool {
    let end = Instant::now() + Duration::from_secs(10);
    while Instant::now() < end {
        if !alive(pid) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

#[test]
fn the_bridge_relays_the_protocol_with_the_user_role() {
    let e = env();
    let mut c = Client::connect_remote(&bridge_spec(&[]), Duration::from_secs(30)).unwrap();
    assert!(c.proto >= 1 && !c.build.is_empty(), "hello result not handed on: proto {} build {:?}", c.proto, c.build);
    let r = c.call("ping", json!({}));
    assert!(r.is_ok(), "ping: {:#}", r.unwrap_err());

    // an agent could not do this: the bridge is not in any session's tree, so it is the user
    let plain = e.root.join("remote-project");
    std::fs::create_dir_all(&plain).unwrap();
    let p = c.call("project.add", json!({"path": &plain})).unwrap();
    let listed = c.call("project.list", json!({})).unwrap().to_string();
    assert!(listed.contains(p["id"].as_str().unwrap()), "{listed}");

    // the bridge always connects as the CLI: no screen control through it
    let err = c.call("computer.enable", json!({"on": true})).unwrap_err().to_string();
    assert!(err.contains("only be turned on by the user"), "{err}");

    // a session runs on the daemon behind the bridge
    let (prog, args) = if cfg!(windows) {
        ("cmd", json!(["/c", "echo hi & ping -n 5 127.0.0.1 >nul"]))
    } else {
        ("sh", json!(["-c", "echo hi; sleep 5"]))
    };
    let id = c.call("session.spawn", json!({"program": prog, "args": args})).unwrap()["id"].as_u64().unwrap();
    let t = Instant::now();
    loop {
        let sessions = c.call("session.list", json!({})).unwrap();
        if sessions.to_string().contains(&format!("\"id\":{id}")) {
            break;
        }
        assert!(t.elapsed() < Duration::from_secs(10), "session {id} not listed: {sessions}");
        std::thread::sleep(Duration::from_millis(100));
    }
    let _ = c.call("session.kill", json!({"id": id}));

    // a refusal by the daemon comes back as the daemon's own error, not as a transport failure, and the link stays usable
    let bad = c.call("project.add", json!({"path": e.root.join("missing")})).unwrap_err();
    assert!(bad.downcast_ref::<axelstra_daemon::client::Remote>().is_some(), "{bad:#}");
    let r = c.call("ping", json!({}));
    assert!(r.is_ok(), "ping: {:#}", r.unwrap_err());

    let pid = c.child_id().unwrap();
    drop(c);
    assert!(wait_dead(pid), "the bridge outlived the client");
}

#[test]
fn a_bridge_with_no_daemon_says_so() {
    let e = env();
    let spec = elsewhere(&e.root.join("other-data"), Some(&"c".repeat(32)));
    let msg = format!("{:#}", Client::connect_remote(&spec, Duration::from_secs(30)).err().expect("must fail"));
    assert!(msg.contains("not usable") && msg.contains("not reachable"), "{msg}");
}

#[test]
fn a_bridge_on_a_machine_that_never_ran_a_daemon_says_so() {
    let e = env();
    let spec = elsewhere(&e.root.join("never-ran"), None);
    let msg = format!("{:#}", Client::connect_remote(&spec, Duration::from_secs(30)).err().expect("must fail"));
    assert!(msg.contains("no daemon has run on this machine"), "{msg}");
}
