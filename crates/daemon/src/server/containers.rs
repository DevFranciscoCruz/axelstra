use super::*;
use std::io::{BufRead, BufReader};
use std::process::Stdio;

/// Does one line of `docker events --format '{{json .}}'` change what the Docker panel shows? Containers, images, volumes
/// and networks do; exec sessions, health probes, attach/resize/top are noise.
pub(super) fn event_changes_state(line: &str) -> bool {
    let Ok(v) = serde_json::from_str::<Value>(line.trim()) else { return false };
    if !matches!(v["Type"].as_str(), Some("container" | "image" | "volume" | "network")) {
        return false;
    }
    let action = v["Action"].as_str().or_else(|| v["status"].as_str()).unwrap_or_default();
    !action.is_empty() && !["exec_", "health_status", "attach", "resize", "top"].iter().any(|n| action.starts_with(n))
}

/// Reads an events stream to its end; `changed` runs for each line that matters. Returns how many lines mattered.
pub(super) fn pump(stream: impl BufRead, mut changed: impl FnMut() -> bool) -> usize {
    let mut n = 0;
    for line in stream.lines() {
        let Ok(line) = line else { break };
        if event_changes_state(&line) {
            n += 1;
            if !changed() {
                break;
            }
        }
    }
    n
}

/// Registries whose docker watcher thread runs (several daemons can live in one test process).
static DOCKER_WATCHES: Mutex<Vec<usize>> = Mutex::new(Vec::new());

const DOCKER_BACKOFF_MIN: Duration = Duration::from_secs(1);
const DOCKER_BACKOFF_MAX: Duration = Duration::from_secs(60);

/// Starts, once per registry, a blocking `docker events` child: each relevant line becomes `docker.changed {}`. The child
/// is restarted when it exits (docker stopped or restarted) with a bounded exponential backoff, and every restart says
/// `docker.changed` too, since docker going up or down changes the panel. Docker not installed ends the watcher; the next
/// `docker.*` call starts it again.
fn ensure_docker_watch(reg: &Arc<Registry>) {
    let key = Arc::as_ptr(reg) as usize;
    {
        let mut w = DOCKER_WATCHES.lock_recover();
        if w.contains(&key) {
            return;
        }
        w.push(key);
    }
    let weak = Arc::downgrade(reg);
    let spawned = std::thread::Builder::new().name("docker-events".into()).spawn(move || {
        let mut delay = DOCKER_BACKOFF_MIN;
        loop {
            let Some(reg) = weak.upgrade() else { break };
            let mut cmd = std::process::Command::new("docker");
            cmd.args(["events", "--format", "{{json .}}"]).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
            let mut child = match axelstra_core::proc::hidden(&mut cmd).spawn() {
                Ok(c) => c,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => break,
                Err(_) => {
                    drop(reg);
                    // kept: bounded exponential backoff while `docker` cannot be started (external program)
                    std::thread::sleep(delay);
                    delay = (delay * 2).min(DOCKER_BACKOFF_MAX);
                    continue;
                }
            };
            let started = Instant::now();
            drop(reg);
            if let Some(out) = child.stdout.take() {
                let w = weak.clone();
                let n = pump(BufReader::new(out), move || w.upgrade().map(|r| r.broadcast("docker.changed")).is_some());
                if n > 0 || started.elapsed() > Duration::from_secs(30) {
                    delay = DOCKER_BACKOFF_MIN;
                }
            }
            let _ = child.kill();
            let _ = child.wait();
            // the stream ended: docker went away or restarted
            let Some(reg) = weak.upgrade() else { break };
            reg.broadcast("docker.changed");
            drop(reg);
            // kept: bounded exponential backoff before re-attaching to `docker events` after docker went away
            std::thread::sleep(delay);
            delay = (delay * 2).min(DOCKER_BACKOFF_MAX);
        }
        DOCKER_WATCHES.lock_recover().retain(|k| *k != key);
    });
    if spawned.is_err() {
        DOCKER_WATCHES.lock_recover().retain(|k| *k != key);
    }
}

pub(super) fn call(c: &mut Ctx) -> Result<Value> {
    let req = c.req;
    let (p, reg) = (c.p, c.reg);
    ensure_docker_watch(reg);
    match req.method.as_str() {
        "docker.open_logs" | "docker.open_shell" => {
            let (s, label, meta) = crate::docker::terminal(&reg.store, &req.method, p)?;
            Ok(json!({"id": reg.spawn(&s, Some(&label), Some(meta))?}))
        }
        "docker.status" | "docker.containers" | "docker.stats" | "docker.images" | "docker.volumes" | "docker.networks"
        | "docker.inspect" | "docker.logs" | "docker.fs_list" | "docker.fs_read" | "docker.action" => {
            tokio::task::block_in_place(|| crate::docker::call(&reg.store, &req.method, p))
        }
        m => Err(unknown_method(m)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_state_changing_events_count() {
        let l = |t: &str, a: &str| format!(r#"{{"Type":"{t}","Action":"{a}","id":"x"}}"#);
        for (t, a) in [("container", "start"), ("container", "die"), ("image", "pull"), ("volume", "create"), ("network", "connect")] {
            assert!(event_changes_state(&l(t, a)), "{t} {a}");
        }
        for (t, a) in [
            ("container", "exec_start: sh"),
            ("container", "exec_die"),
            ("container", "health_status: healthy"),
            ("container", "attach"),
            ("container", "resize"),
            ("container", "top"),
            ("daemon", "reload"),
            ("plugin", "enable"),
        ] {
            assert!(!event_changes_state(&l(t, a)), "{t} {a}");
        }
        assert!(!event_changes_state(""));
        assert!(!event_changes_state("not json"));
        assert!(!event_changes_state(r#"{"Type":"container"}"#));
        // older engines name the action `status`
        assert!(event_changes_state(r#"{"Type":"container","status":"start"}"#));
    }

    #[test]
    fn pump_counts_relevant_lines_and_stops_when_asked() {
        let text = [
            r#"{"Type":"container","Action":"create"}"#,
            r#"{"Type":"container","Action":"exec_start: ls"}"#,
            "garbage",
            r#"{"Type":"network","Action":"connect"}"#,
            r#"{"Type":"container","Action":"start"}"#,
        ]
        .join(
            "
",
        );
        let mut seen = 0;
        let n = pump(text.as_bytes(), || {
            seen += 1;
            true
        });
        assert_eq!((n, seen), (3, 3));
        // `false` (the registry is gone) ends the stream at once
        assert_eq!(pump(text.as_bytes(), || false), 1);
    }
}
