//! A pending call fails only by event: the reply, the connection closing, or the process behind it going away. Never by
//! elapsed time (except an explicit `Client::timeout`, a cosmetic method's policy, or the high safety ceiling).
//! The fake daemons here are gated by the test (a `Notify`), so "a slow daemon" needs no long sleep.

mod common;
use axelstra_daemon::client::{self, CallTimeout, Client};
use axelstra_daemon::proto::{encode, JSON};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::Notify;

#[derive(Clone)]
enum Mode {
    /// answers everything at once
    Echo,
    /// answers `hello`, then holds every other reply until the gate opens
    Gated(Arc<Notify>),
    /// answers `hello`, then drops the connection on the next request without replying
    DropMid,
}

async fn read_req(conn: &mut (impl AsyncRead + Unpin)) -> Option<Value> {
    let mut len = [0u8; 4];
    conn.read_exact(&mut len).await.ok()?;
    let mut buf = vec![0u8; u32::from_le_bytes(len) as usize];
    conn.read_exact(&mut buf).await.ok()?;
    serde_json::from_slice(&buf[1..]).ok()
}

async fn reply(conn: &mut (impl AsyncWrite + Unpin), req: &Value, result: Value) {
    let body = json!({"id": req["id"], "result": result});
    let _ = conn.write_all(&encode(JSON, body.to_string().as_bytes())).await;
}

async fn behave(mut conn: impl AsyncRead + AsyncWrite + Unpin, mode: Mode) {
    while let Some(req) = read_req(&mut conn).await {
        let hello = req["method"] == "hello";
        let result = if hello { json!({"proto": 1, "instance": 1}) } else { json!("pong") };
        match &mode {
            Mode::Echo => reply(&mut conn, &req, result).await,
            Mode::Gated(gate) => {
                if !hello {
                    gate.notified().await;
                }
                reply(&mut conn, &req, result).await;
            }
            Mode::DropMid => {
                if hello {
                    reply(&mut conn, &req, result).await;
                } else {
                    return; // dropping `conn` closes the pipe mid-call
                }
            }
        }
    }
}

/// Serves one connection per entry of `modes`, in order, on `pipe`; returns once the first is listening.
fn serve(pipe: String, modes: Vec<Mode>) {
    let (up, ready) = std::sync::mpsc::channel::<()>();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
        rt.block_on(async move {
            #[cfg(windows)]
            {
                use tokio::net::windows::named_pipe::ServerOptions;
                let mut next = ServerOptions::new().first_pipe_instance(true).create(&pipe).unwrap();
                up.send(()).unwrap();
                for m in modes {
                    let cur = next;
                    // the next instance exists before this one is served, so a client reconnecting at once finds it
                    next = ServerOptions::new().create(&pipe).unwrap();
                    cur.connect().await.unwrap();
                    tokio::spawn(behave(cur, m));
                }
                std::future::pending::<()>().await;
            }
            #[cfg(unix)]
            {
                let _ = std::fs::remove_file(&pipe);
                let l = tokio::net::UnixListener::bind(&pipe).unwrap();
                up.send(()).unwrap();
                for m in modes {
                    let (conn, _) = l.accept().await.unwrap();
                    tokio::spawn(behave(conn, m));
                }
                std::future::pending::<()>().await;
            }
        });
    });
    ready.recv_timeout(Duration::from_secs(30)).expect("the fake daemon never started listening");
}

fn code(e: &anyhow::Error) -> Option<String> {
    axelstra_core::msg::find(e).map(|c| c.code.clone())
}

/// A stuck test must not hang the suite.
fn suite_watchdog() {
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(120));
        eprintln!("watchdog: a call never ended");
        std::process::exit(1);
    });
}

#[test]
fn no_method_has_a_fixed_default_limit() {
    for m in ["ping", "session.list", "git.log", "git.push", "worktree.overview", "orch.start", "diagnostics", "project.list"] {
        assert_eq!(client::policy_limit(m), None, "{m}");
    }
    // only a cosmetic read keeps a policy limit
    assert!(client::policy_limit("project.logo").is_some());
    // the ceiling is high: well above the old 30/120 s defaults
    assert!(client::ceiling() >= Duration::from_secs(60));
}

#[test]
fn a_slow_reply_completes_with_success_when_nothing_closes() {
    suite_watchdog();
    let pipe = common::pipe("slow");
    let gate = Arc::new(Notify::new());
    serve(pipe.clone(), vec![Mode::Gated(gate.clone())]);
    let mut c = Client::connect(&pipe, "t").unwrap();
    assert_eq!(c.timeout, None);
    // the reply comes when the gate opens, long after any explicit limit a test would set and nowhere near the ceiling
    let opener = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(1500));
        gate.notify_one();
    });
    let t0 = Instant::now();
    let r = c.call("git.log", json!({})).expect("a slow reply is a success");
    assert_eq!(r, json!("pong"));
    assert!(t0.elapsed() >= Duration::from_millis(1400), "answered before the gate opened: {:?}", t0.elapsed());
    opener.join().unwrap();
}

#[test]
fn a_connection_closed_mid_call_fails_at_once() {
    suite_watchdog();
    let pipe = common::pipe("dropmid");
    serve(pipe.clone(), vec![Mode::DropMid, Mode::Echo]);
    let mut c = Client::connect(&pipe, "t").unwrap();
    let t0 = Instant::now();
    let err = c.call("git.status", json!({})).unwrap_err();
    assert!(t0.elapsed() < Duration::from_secs(10), "took {:?}", t0.elapsed());
    assert_eq!(code(&err).as_deref(), Some("rpc.connection_closed"), "{err:#}");
    assert!(err.downcast_ref::<CallTimeout>().is_none(), "a close is not a timeout: {err:#}");
    assert!(format!("{err:#}").contains("git.status"), "the method is named: {err:#}");
    // that connection is finished; the next call on it says so at once
    assert_eq!(code(&c.call("ping", json!({})).unwrap_err()).as_deref(), Some("rpc.connection_closed"));
    // and a fresh connection works
    let mut again = Client::connect(&pipe, "t").unwrap();
    assert_eq!(again.call("ping", json!({})).unwrap(), json!("pong"));
}

#[test]
fn the_ceiling_ends_a_lost_handle_and_the_next_connection_works() {
    suite_watchdog();
    let pipe = common::pipe("ceiling");
    serve(pipe.clone(), vec![Mode::Gated(Arc::new(Notify::new())), Mode::Echo]);
    let mut c = Client::connect(&pipe, "t").unwrap();
    c.ceiling = Duration::from_millis(400);
    let t0 = Instant::now();
    let err = c.call("git.log", json!({})).unwrap_err();
    assert!(t0.elapsed() < Duration::from_secs(10), "took {:?}", t0.elapsed());
    assert!(err.downcast_ref::<CallTimeout>().is_some(), "{err:#}");
    assert_eq!(code(&err).as_deref(), Some("rpc.call_ceiling"), "{err:#}");
    let mut again = Client::connect(&pipe, "t").unwrap();
    assert_eq!(again.call("ping", json!({})).unwrap(), json!("pong"));
}

#[test]
fn an_explicit_timeout_is_still_honoured_and_coded_as_such() {
    suite_watchdog();
    let pipe = common::pipe("explicit");
    serve(pipe.clone(), vec![Mode::Gated(Arc::new(Notify::new()))]);
    let mut c = Client::connect(&pipe, "t").unwrap();
    c.timeout = Some(Duration::from_millis(300));
    let err = c.call("ping", json!({})).unwrap_err();
    assert_eq!(code(&err).as_deref(), Some("rpc.call_timeout"), "{err:#}");
}
