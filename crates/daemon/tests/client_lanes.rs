//! One slow reply must not hold up the other calls of the app: the daemon runs the calls of a connection in order, so
//! the app uses several connections (`Lanes`) and keeps one for quick methods.

mod common;
use axelstra_daemon::client::{Client, Lanes};
use axelstra_daemon::proto::{encode, JSON};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWrite, AsyncWriteExt};

const SLOW: Duration = Duration::from_millis(1500);

/// Like the real daemon: answers the requests of one connection one after the other; `slow` takes `SLOW`.
async fn serial_daemon(mut conn: impl tokio::io::AsyncRead + AsyncWrite + Unpin) {
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
        let result = match req["method"].as_str().unwrap() {
            "hello" => json!({"proto": 1, "instance": 1}),
            "slow" => {
                tokio::time::sleep(SLOW).await;
                json!("slow")
            }
            _ => json!("fast"),
        };
        let body = json!({"id": req["id"], "result": result});
        if conn.write_all(&encode(JSON, body.to_string().as_bytes())).await.is_err() {
            return;
        }
    }
}

fn serve(pipe: String) {
    let (up, ready) = std::sync::mpsc::channel::<()>();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(async move {
            #[cfg(windows)]
            {
                use tokio::net::windows::named_pipe::ServerOptions;
                let mut server = ServerOptions::new().first_pipe_instance(true).create(&pipe).unwrap();
                up.send(()).unwrap();
                loop {
                    server.connect().await.unwrap();
                    let next = ServerOptions::new().create(&pipe).unwrap();
                    tokio::spawn(serial_daemon(std::mem::replace(&mut server, next)));
                }
            }
            #[cfg(unix)]
            {
                let _ = std::fs::remove_file(&pipe);
                let l = tokio::net::UnixListener::bind(&pipe).unwrap();
                up.send(()).unwrap();
                loop {
                    let (conn, _) = l.accept().await.unwrap();
                    tokio::spawn(serial_daemon(conn));
                }
            }
        });
    });
    ready.recv_timeout(Duration::from_secs(30)).expect("the fake daemon never started listening");
}

/// The old shape: one connection behind one lock. A quick call waits for the whole slow one.
#[test]
fn one_connection_makes_a_quick_call_wait_for_a_slow_one() {
    let pipe = common::pipe("lane-one");
    serve(pipe.clone());
    let c = Arc::new(Mutex::new(Client::connect(&pipe, "t").unwrap()));
    let slow = {
        let c = c.clone();
        std::thread::spawn(move || c.lock().unwrap().call("slow", json!({})).unwrap())
    };
    std::thread::sleep(Duration::from_millis(200));
    let t0 = Instant::now();
    let r = c.lock().unwrap().call("fast", json!({})).unwrap();
    assert_eq!(r, json!("fast"));
    assert!(t0.elapsed() >= SLOW - Duration::from_millis(400), "the quick call did not wait: {:?}", t0.elapsed());
    slow.join().unwrap();
}

fn call_on(lanes: &Lanes<Client>, pipe: &str, method: &str, slow: bool) -> Value {
    let claim = lanes.pick(slow);
    let mut g = claim.slot().lock().unwrap();
    if g.is_none() {
        *g = Some(Client::connect(pipe, "t").unwrap());
    }
    g.as_mut().unwrap().call(method, json!({})).unwrap()
}

/// The pool: a quick call is answered at once while two slow ones run, and the two slow ones run side by side.
#[test]
fn a_slow_call_does_not_hold_up_quick_ones() {
    let pipe = common::pipe("lane-pool");
    serve(pipe.clone());
    let lanes = Arc::new(Lanes::<Client>::new(3, 1));
    let t0 = Instant::now();
    let slows: Vec<_> = (0..2)
        .map(|_| {
            let (l, p) = (lanes.clone(), pipe.clone());
            std::thread::spawn(move || call_on(&l, &p, "slow", true))
        })
        .collect();
    std::thread::sleep(Duration::from_millis(200));
    let q0 = Instant::now();
    assert_eq!(call_on(&lanes, &pipe, "fast", false), json!("fast"));
    assert!(q0.elapsed() < Duration::from_millis(700), "quick call waited {:?}", q0.elapsed());
    for s in slows {
        assert_eq!(s.join().unwrap(), json!("slow"));
    }
    // two slow calls took about one `SLOW`, not two
    assert!(t0.elapsed() < SLOW * 2 - Duration::from_millis(300), "slow calls ran one after the other: {:?}", t0.elapsed());
}

#[test]
fn slow_methods_never_take_the_reserved_slot() {
    let lanes = Lanes::<u8>::new(3, 1);
    let a = lanes.pick(true);
    let b = lanes.pick(true);
    // both allowed slots are busy: a third slow call queues on one of them, the reserved slot stays free
    let c = lanes.pick(true);
    assert!(!std::ptr::eq(a.slot(), b.slot()));
    assert!(std::ptr::eq(c.slot(), a.slot()) || std::ptr::eq(c.slot(), b.slot()));
    let quick = lanes.pick(false);
    assert!(!std::ptr::eq(quick.slot(), a.slot()) && !std::ptr::eq(quick.slot(), b.slot()));
}
