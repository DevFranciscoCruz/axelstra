//! A remote daemon is a child process (ssh / wsl.exe running `axelstra bridge`): deadlines kill it, dropping the client
//! kills it, and what it says on stderr reaches the error. The child here is this very test binary playing the bridge
//! (`helper` below, run only when AXELSTRA_TEST_HELPER is set), so nothing needs ssh or WSL.

use axelstra_daemon::client::{CallTimeout, Client};
use axelstra_daemon::proto::{encode, JSON};
use axelstra_daemon::remote::RemoteSpec;
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::time::{Duration, Instant};

/// The child side. Modes: `hung` (noise, banner, answers hello, then never answers anything),
/// `crash` (stderr text, exit 127), `silent` (noise only, never announces itself).
#[test]
fn helper() {
    let Ok(mode) = std::env::var("AXELSTRA_TEST_HELPER") else { return };
    let (mut out, mut inp) = (std::io::stdout(), std::io::stdin());
    match mode.as_str() {
        "crash" => {
            eprintln!("bash: axelstra: command not found");
            std::process::exit(127);
        }
        "silent" => {
            out.write_all(b"Welcome, this login script talks too much\n").unwrap();
            out.flush().unwrap();
            std::thread::sleep(Duration::from_secs(120));
        }
        _ => {
            eprintln!("hung helper started");
            out.write_all(b"Welcome to a box\r\nlast login: yesterday\nAXELSTRA-BRIDGE/1\n").unwrap();
            out.flush().unwrap();
            loop {
                let mut len = [0u8; 4];
                if inp.read_exact(&mut len).is_err() {
                    return;
                }
                let mut buf = vec![0u8; u32::from_le_bytes(len) as usize];
                if inp.read_exact(&mut buf).is_err() {
                    return;
                }
                let req: Value = serde_json::from_slice(&buf[1..]).unwrap();
                if req["method"] == "hello" {
                    let body = json!({"id": req["id"], "result": {"proto": 7, "instance": 3, "build": "t", "security": 2}});
                    out.write_all(&encode(JSON, body.to_string().as_bytes())).unwrap();
                    out.flush().unwrap();
                }
            }
        }
    }
}

fn helper_spec(mode: &str) -> RemoteSpec {
    RemoteSpec::Exec {
        program: std::env::current_exe().unwrap().to_string_lossy().into_owned(),
        args: ["--exact", "helper", "--nocapture", "--test-threads=1"].map(String::from).to_vec(),
        env: vec![("AXELSTRA_TEST_HELPER".into(), mode.into())],
    }
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
    let end = Instant::now() + Duration::from_secs(5);
    while Instant::now() < end {
        if !alive(pid) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

#[test]
fn hello_works_through_login_noise_and_a_silent_call_kills_the_child() {
    let mut c = Client::connect_remote(&helper_spec("hung"), Duration::from_secs(20)).unwrap();
    assert_eq!((c.proto, c.instance, c.security), (7, 3, 2));
    let pid = c.child_id().unwrap();
    assert!(alive(pid));
    c.timeout = Some(Duration::from_millis(400));
    let t0 = Instant::now();
    let err = c.call("ping", json!({})).unwrap_err();
    assert!(t0.elapsed() < Duration::from_secs(10), "took {:?}", t0.elapsed());
    assert!(err.downcast_ref::<CallTimeout>().is_some(), "not a CallTimeout: {err:#}");
    assert!(format!("{err:#}").contains("hung helper started"), "stderr of the child missing: {err:#}");
    assert!(wait_dead(pid), "the child outlived its call timeout");
    // and the connection is not reused
    assert!(c.call("ping", json!({})).unwrap_err().downcast_ref::<CallTimeout>().is_some());
}

#[test]
fn dropping_the_client_kills_the_child() {
    let c = Client::connect_remote(&helper_spec("hung"), Duration::from_secs(20)).unwrap();
    let pid = c.child_id().unwrap();
    assert!(alive(pid));
    drop(c);
    assert!(wait_dead(pid), "the child outlived the client");
}

#[test]
fn a_clone_keeps_the_child_alive_until_the_last_one_is_dropped() {
    let c = Client::connect_remote(&helper_spec("hung"), Duration::from_secs(20)).unwrap();
    let pid = c.child_id().unwrap();
    let c2 = c.try_clone().unwrap();
    drop(c);
    assert!(alive(pid));
    drop(c2);
    assert!(wait_dead(pid));
}

#[test]
fn a_child_that_fails_says_why() {
    let e = Client::connect_remote(&helper_spec("crash"), Duration::from_secs(20)).err().expect("must fail");
    let msg = format!("{e:#}");
    assert!(msg.contains("command not found"), "{msg}");
    assert!(msg.contains("could not reach"), "{msg}");
}

#[test]
fn a_bridge_that_never_announces_itself_is_given_up_on() {
    let t0 = Instant::now();
    let e = Client::connect_remote(&helper_spec("silent"), Duration::from_millis(700)).err().expect("must fail");
    assert!(t0.elapsed() < Duration::from_secs(10), "took {:?}", t0.elapsed());
    let msg = format!("{e:#}");
    assert!(msg.contains("no answer within"), "{msg}");
}

#[test]
fn a_missing_program_is_named() {
    let spec = RemoteSpec::Exec { program: "axelstra-no-such-program-xyz".into(), args: vec![], env: vec![] };
    let e = Client::connect_remote(&spec, Duration::from_secs(5)).err().expect("must fail");
    assert!(format!("{e:#}").contains("could not start `axelstra-no-such-program-xyz`"), "{e:#}");
}
