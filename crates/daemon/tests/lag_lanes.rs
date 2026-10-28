//! A slow git-like call must not delay the fast ones, on the same connection or another, and identical calls in
//! flight share one run. `debug.slow` (only with AXELSTRA_TESTING) stands in for a slow git.
mod common;
use axelstra_daemon::client::Client;
use axelstra_daemon::server;
use serde_json::json;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

fn sh(cwd: &Path, args: &[&str]) {
    assert!(Command::new("git").current_dir(cwd).args(args).status().unwrap().success(), "git {args:?}");
}

fn start(tag: &str) -> (String, String, std::path::PathBuf) {
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(120));
        eprintln!("watchdog: test stuck");
        std::process::exit(1);
    });
    let root = std::env::temp_dir().join(format!("axelstra-lag-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let pipe = format!("{}-lag-{tag}-{}", common::base_pipe(), std::process::id());
    let token = "l".repeat(32);
    let (p, t, s) = (pipe.clone(), token.clone(), root.join("projects.json"));
    std::thread::spawn(move || {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap()
            .block_on(server::serve_with_store(p, t, s))
            .unwrap();
    });
    common::wait_ready(&pipe, &token);
    (pipe, token, root)
}

fn ms(t: Instant) -> u128 {
    t.elapsed().as_millis()
}

#[test]
fn fast_methods_do_not_wait_for_slow_ones_on_the_same_connection() {
    let (pipe, token, root) = start("fast");
    let mut c = Client::connect(&pipe, &token).unwrap();
    for n in 0..3 {
        let repo = root.join(format!("r{n}"));
        std::fs::create_dir_all(&repo).unwrap();
        sh(&repo, &["init", "-q", "-b", "main"]);
        sh(&repo, &["config", "user.email", "t@t"]);
        sh(&repo, &["config", "user.name", "t"]);
        std::fs::write(repo.join("a.txt"), "x").unwrap();
        sh(&repo, &["add", "a.txt"]);
        sh(&repo, &["commit", "-qm", "init"]);
        c.call("project.add", json!({"path": &repo})).unwrap();
    }
    // four slow calls in flight (distinct params: no coalescing), 2 s each
    for n in 0..4 {
        c.send("debug.slow", json!({"ms": 2000 + n})).unwrap();
    }
    let t = Instant::now();
    c.call("ping", json!({})).unwrap();
    let ping = ms(t);
    let t = Instant::now();
    c.call("settings.get", json!({})).unwrap();
    let settings = ms(t);
    let t = Instant::now();
    c.call("project.list", json!({})).unwrap();
    let plist = ms(t);
    eprintln!("LAG-NUMBERS ping={ping}ms settings.get={settings}ms project.list(cold)={plist}ms");
    assert!(ping < 300, "ping waited {ping} ms behind slow calls");
    assert!(settings < 300, "settings.get waited {settings} ms");
    assert!(plist < 1500, "project.list waited {plist} ms");
}

#[test]
fn identical_calls_in_flight_share_one_run() {
    let (pipe, token, _root) = start("flight");
    let handles: Vec<_> = (0..5)
        .map(|_| {
            let (p, t) = (pipe.clone(), token.clone());
            std::thread::spawn(move || {
                let mut c = Client::connect(&p, &t).unwrap();
                let t0 = Instant::now();
                let r = c.call("debug.slow", json!({"ms": 700})).unwrap();
                (r["ran"].as_u64().unwrap(), t0.elapsed())
            })
        })
        .collect();
    let runs: Vec<u64> = handles.into_iter().map(|h| h.join().unwrap().0).collect();
    eprintln!("LAG-NUMBERS executions={:?}", runs);
    assert!(runs.windows(2).all(|w| w[0] == w[1]), "five identical calls ran separately: {runs:?}");
}

#[test]
fn project_list_right_after_project_add_lists_it_even_beside_another_daemon_and_in_flight_lists() {
    let (pa, ta, _ra) = start("plist-a");
    let (pb, tb, rb) = start("plist-b");
    // daemon A keeps asking for its (empty) list: its runs must never answer daemon B
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let s2 = stop.clone();
    let hammer = std::thread::spawn(move || {
        let mut a = Client::connect(&pa, &ta).unwrap();
        while !s2.load(std::sync::atomic::Ordering::SeqCst) {
            a.call("project.list", json!({})).unwrap();
        }
    });
    let mut b = Client::connect(&pb, &tb).unwrap();
    let mut b2 = Client::connect(&pb, &tb).unwrap();
    for n in 0..6 {
        let repo = rb.join(format!("r{n}"));
        std::fs::create_dir_all(&repo).unwrap();
        sh(&repo, &["init", "-q", "-b", "main"]);
        sh(&repo, &["config", "user.email", "t@t"]);
        sh(&repo, &["config", "user.name", "t"]);
        std::fs::write(repo.join("a.txt"), "x").unwrap();
        sh(&repo, &["add", "a.txt"]);
        sh(&repo, &["commit", "-qm", "init"]);
        // a list in flight on the same daemon while the project is added
        let t = std::thread::spawn(move || {
            let _ = b2.call("project.list", json!({}));
            b2
        });
        b.call("project.add", json!({"path": &repo})).unwrap();
        let l = b.call("project.list", json!({})).unwrap();
        assert_eq!(l.as_array().map(|a| a.len()), Some(n + 1), "list right after add #{n}: {l}");
        b2 = t.join().unwrap();
    }
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    hammer.join().unwrap();
}
