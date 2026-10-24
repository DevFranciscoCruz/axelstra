//! A daemon that stops answering must give the caller an error, not a frozen thread.

mod common;
use axelstra_daemon::client::{CallTimeout, Client};
use axelstra_daemon::proto::{encode, JSON};
use serde_json::{json, Value};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Answers `hello`, then reads every later request and never replies.
async fn hung_daemon(mut conn: impl tokio::io::AsyncRead + AsyncWrite + Unpin) {
    loop {
        let mut len = [0u8; 4];
        if conn.read_exact(&mut len).await.is_err() {
            return;
        }
        let mut buf = vec![0u8; u32::from_le_bytes(len) as usize];
        if conn.read_exact(&mut buf).await.is_err() {
            return;
        }
        let req: Value = serde_json::from_slice(&buf[1..]).unwrap();
        if req["method"] == "hello" {
            let body = json!({"id": req["id"], "result": {"proto": 1, "instance": 1}});
            let _ = conn.write_all(&encode(JSON, body.to_string().as_bytes())).await;
        }
    }
}

/// Starts the hung daemon and returns once its pipe exists, so the caller connects to a listener that is there.
fn serve_hung(pipe: String) {
    let (up, ready) = std::sync::mpsc::channel::<()>();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(async move {
            #[cfg(windows)]
            {
                let server = tokio::net::windows::named_pipe::ServerOptions::new().first_pipe_instance(true).create(&pipe).unwrap();
                up.send(()).unwrap();
                server.connect().await.unwrap();
                hung_daemon(server).await;
            }
            #[cfg(unix)]
            {
                let _ = std::fs::remove_file(&pipe);
                let l = tokio::net::UnixListener::bind(&pipe).unwrap();
                up.send(()).unwrap();
                let (conn, _) = l.accept().await.unwrap();
                hung_daemon(conn).await;
            }
        });
    });
    ready.recv_timeout(Duration::from_secs(30)).expect("the hung daemon never started listening");
}

#[test]
fn a_hung_daemon_times_out_with_a_clear_error() {
    let pipe = format!("{}-hung-{}", common::base_pipe(), std::process::id());
    serve_hung(pipe.clone());
    // a stuck test must not hang the suite
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(60));
        eprintln!("watchdog: timeout never fired");
        std::process::exit(1);
    });

    let mut c = Client::connect(&pipe, "t").unwrap();
    c.timeout = Some(Duration::from_millis(300));
    let t0 = Instant::now();
    let err = c.call("ping", json!({})).unwrap_err();
    assert!(t0.elapsed() < Duration::from_secs(10), "took {:?}", t0.elapsed());
    let timeout = err.downcast_ref::<CallTimeout>().unwrap_or_else(|| panic!("not a CallTimeout: {err:#}"));
    assert!(timeout.0.contains("ping") && timeout.0.contains("did not answer"), "{timeout}");

    // the connection is not reused after a timeout
    let again = c.call("ping", json!({})).unwrap_err();
    assert!(again.downcast_ref::<CallTimeout>().is_some(), "{again:#}");
}
