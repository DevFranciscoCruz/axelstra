//! `project.logo` runs in its own lane with a cache (also of "no logo"): repeated asks do not walk the disk, a burst of
//! them never delays a cheap call, and the answer always comes well inside the app's 30 s limit. The daemon makes no
//! network request for a logo (the host avatar is fetched by the webview), so there is no upstream to stall here.
mod common;
use axelstra_daemon::client::Client;
use axelstra_daemon::server;
use serde_json::json;
use std::time::{Duration, Instant};

#[test]
fn logo_is_cached_and_never_delays_other_calls() {
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(60));
        eprintln!("watchdog: test stuck");
        std::process::exit(1);
    });
    let root = std::env::temp_dir().join(format!("axelstra-logo-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let dir = root.join("plain");
    std::fs::create_dir_all(&dir).unwrap();
    let pipe = format!("{}-logo-{}", common::base_pipe(), std::process::id());
    let token = "g".repeat(32);
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
    let mut c = Client::connect(&pipe, &token).unwrap();
    let id = c.call("project.add", json!({"path": &dir})).unwrap()["id"].as_str().unwrap().to_string();

    // no logo: null, and remembered
    let t0 = Instant::now();
    assert!(c.call("project.logo", json!({"id": id})).unwrap()["data_url"].is_null());
    assert!(t0.elapsed() < Duration::from_secs(7));
    std::fs::write(dir.join("logo.png"), [137u8, 80, 78, 71]).unwrap();
    assert!(c.call("project.logo", json!({"id": id})).unwrap()["data_url"].is_null(), "served from the negative cache");

    // after the cache is dropped the file shows; then it is served from the cache even with the file gone
    axelstra_daemon::projects::forget_logos();
    assert!(c.call("project.logo", json!({"id": id})).unwrap()["data_url"].as_str().unwrap().starts_with("data:image/png;base64,"));
    std::fs::remove_file(dir.join("logo.png")).unwrap();
    assert!(c.call("project.logo", json!({"id": id})).unwrap()["data_url"].is_string(), "served from the cache");

    // a burst of logo calls on other connections does not delay a ping
    axelstra_daemon::projects::forget_logos();
    let hs: Vec<_> = (0..8)
        .map(|_| {
            let (pipe, token, id) = (pipe.clone(), token.clone(), id.clone());
            std::thread::spawn(move || {
                let mut c = Client::connect(&pipe, &token).unwrap();
                for _ in 0..20 {
                    c.call("project.logo", json!({"id": id})).unwrap();
                }
            })
        })
        .collect();
    let t0 = Instant::now();
    c.call("ping", json!({})).unwrap();
    assert!(t0.elapsed() < Duration::from_secs(2), "ping took {:?}", t0.elapsed());
    for h in hs {
        h.join().unwrap();
    }
    let _ = std::fs::remove_dir_all(&root);
}
