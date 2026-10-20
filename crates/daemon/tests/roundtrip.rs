
use axelstra_daemon::client::{Client, Frame};
use axelstra_daemon::server;
use serde_json::json;
use std::time::{Duration, Instant};

#[cfg(windows)]
const SHELL: &str = "cmd";
#[cfg(windows)]
const SHELL_ARGS: [&str; 1] = ["/q"];
#[cfg(not(windows))]
const SHELL: &str = "sh";
#[cfg(not(windows))]
const SHELL_ARGS: [&str; 0] = [];

/// `echo <a>` then `echo <b>` on one line, the way each shell chains commands
fn two_echoes(a: &str, b: &str) -> Vec<u8> {
    let sep = if cfg!(windows) { "&" } else { ";" };
    let eol = if cfg!(windows) { "\r\n" } else { "\n" };
    format!("echo {a}{sep}echo {b}{eol}").into_bytes()
}

fn start() -> (String, String) {
    // blocking pipe reads cannot time out, so a stuck test must not hang the suite
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(90));
        eprintln!("watchdog: test stuck");
        std::process::exit(1);
    });
    let pipe = format!("{}-test-{}-{:?}", axelstra_daemon::proto::pipe_name(), std::process::id(), std::thread::current().id())
        .replace("ThreadId(", "")
        .replace(')', "");
    let token = "t".repeat(32);
    let (p, t) = (pipe.clone(), token.clone());
    std::thread::spawn(move || {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap()
            .block_on(server::serve(p, t))
            .unwrap();
    });
    std::thread::sleep(Duration::from_millis(300));
    (pipe, token)
}

fn collect(c: &mut Client, id: u32, needle: &str) -> bool {
    let mut all = Vec::new();
    let end = Instant::now() + Duration::from_secs(10);
    while Instant::now() < end {
        if let Ok(Frame::Output { session, data }) = c.read() {
            if session == id {
                all.extend(data);
                if String::from_utf8_lossy(&all).contains(needle) {
                    return true;
                }
            }
        }
    }
    false
}

#[test]
fn rejects_wrong_token() {
    let (pipe, _) = start();
    assert!(Client::connect(&pipe, "nope").is_err());
}

#[test]
fn session_survives_disconnect_and_replays_to_new_client() {
    let (pipe, token) = start();

    let mut a = Client::connect(&pipe, &token).unwrap();
    let id = a
        .call("session.spawn", json!({"program": SHELL, "args": SHELL_ARGS, "cols": 80, "rows": 24}))
        .unwrap()["id"]
        .as_u64()
        .unwrap() as u32;
    a.call("session.subscribe", json!({"id": id})).unwrap();
    a.send_input(id, &two_echoes("first", "-marker")).unwrap();
    assert!(collect(&mut a, id, "first"));
    drop(a); // UI goes away

    let mut b = Client::connect(&pipe, &token).unwrap();
    let list = b.call("session.list", json!({})).unwrap();
    assert_eq!(list[0]["running"], true);
    b.call("session.subscribe", json!({"id": id})).unwrap();
    // snapshot replays what happened while nobody was attached
    assert!(collect(&mut b, id, "first"));
    b.send_input(id, &two_echoes("second", "x")).unwrap();
    assert!(collect(&mut b, id, "second"));

    let u = b.call("session.usage", json!({"id": id})).unwrap();
    assert!(u["peak_mem_bytes"].as_u64().unwrap() > 0);

    b.call("session.kill", json!({"id": id})).unwrap();
    let end = Instant::now() + Duration::from_secs(5);
    let mut exited = false;
    while Instant::now() < end {
        if let Ok(Frame::Event(e)) = b.read() {
            if e["event"] == "session.exit" {
                exited = true;
                break;
            }
        }
    }
    assert!(exited, "no session.exit event");
}
