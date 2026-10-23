//! A small Docker Desktop on top of the `docker` CLI. Every call is a fixed argument list (never a shell), has a timeout,
//! and every id, name and image reference is checked before it reaches the command line.

use anyhow::{bail, Context, Result};
use axelstra_core::git;
use axelstra_core::pty::Spawn;
use serde::Serialize;
use serde_json::{json, Value};
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const QUERY: Duration = Duration::from_secs(10);
const ACTION: Duration = Duration::from_secs(60);
const COMPOSE_FILES: [&str; 4] = ["compose.yaml", "compose.yml", "docker-compose.yml", "docker-compose.yaml"];
/// what a container with `docker exec` runs: bash when the image has it, plain sh otherwise
const SHELL_SCRIPT: &str = "if command -v bash >/dev/null 2>&1; then exec bash; else exec sh; fi";

#[derive(Serialize, Debug, PartialEq)]
pub struct Status {
    pub installed: bool,
    pub running: bool,
    pub version: String,
    pub error: String,
}

#[derive(Serialize, Debug, PartialEq, Clone)]
pub struct Port {
    /// published port on the host, absent for a port that is only exposed
    pub host: Option<u16>,
    pub container: u16,
    pub proto: String,
}

#[derive(Serialize, Debug, PartialEq, Clone)]
pub struct Container {
    pub id: String,
    pub name: String,
    pub image: String,
    /// running, exited, paused, created, restarting, dead
    pub state: String,
    pub status: String,
    pub ports: Vec<Port>,
    pub created: String,
    pub project: String,
    pub working_dir: String,
    pub service: String,
    /// names of the volumes it mounts (bind mounts are left out)
    pub volumes: Vec<String>,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct Stat {
    pub id: String,
    pub name: String,
    /// percent, 12.5 for "12.5%"
    pub cpu: f64,
    /// "571.4MiB / 15.2GiB"
    pub mem: String,
    pub mem_pct: f64,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct Image {
    pub id: String,
    pub tags: Vec<String>,
    pub size: String,
    pub created: String,
    pub containers: u32,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct Volume {
    pub name: String,
    pub driver: String,
    pub mountpoint: String,
    pub used_by: Vec<String>,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct Project {
    pub name: String,
    pub working_dir: String,
    pub running: usize,
    pub total: usize,
}

/// Docker's own names: container and volume names, ids and compose project names.
pub fn valid_name(s: &str) -> bool {
    let mut c = s.chars();
    s.len() <= 255 && c.next().is_some_and(|f| f.is_ascii_alphanumeric()) && c.all(|x| x.is_ascii_alphanumeric() || matches!(x, '_' | '.' | '-'))
}

/// An image reference (`repo/name:tag`, `name@sha256:...`) or id: the name charset plus `/`, `:` and `@`.
pub fn valid_reference(s: &str) -> bool {
    let mut c = s.chars();
    s.len() <= 255 && c.next().is_some_and(|f| f.is_ascii_alphanumeric()) && c.all(|x| x.is_ascii_alphanumeric() || matches!(x, '_' | '.' | '-' | '/' | ':' | '@'))
}

fn name_arg<'a>(kind: &str, s: &'a str) -> Result<&'a str> {
    if !valid_name(s) {
        bail!("invalid {kind} {s:?}: only letters, digits, _ . - are allowed");
    }
    Ok(s)
}

/// Runs docker; stdout on success, the trimmed stderr as the error. `NotFound` means docker is not installed.
fn run(args: &[&str], limit: Duration) -> Result<String> {
    let mut child = git::quiet(&mut Command::new("docker"))
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| if e.kind() == std::io::ErrorKind::NotFound { anyhow::anyhow!("docker is not installed") } else { anyhow::Error::new(e).context("could not start docker") })?;
    // drained on their own threads: a full pipe would block docker before the timeout could matter
    let mut out = child.stdout.take().unwrap();
    let mut err = child.stderr.take().unwrap();
    let out_t = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = out.read_to_string(&mut s);
        s
    });
    let err_t = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = err.read_to_string(&mut s);
        s
    });
    let start = Instant::now();
    let code = loop {
        if let Some(st) = child.try_wait()? {
            break st;
        }
        if start.elapsed() > limit {
            let _ = child.kill();
            let _ = child.wait();
            bail!("docker {} timed out after {}s", args.first().unwrap_or(&""), limit.as_secs());
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let (stdout, stderr) = (out_t.join().unwrap_or_default(), err_t.join().unwrap_or_default());
    if !code.success() {
        let msg = stderr.trim();
        bail!("{}", if msg.is_empty() { format!("docker {} failed", args.first().unwrap_or(&"")) } else { msg.to_string() });
    }
    Ok(stdout)
}

/// Not installed and engine not running are different problems with different fixes.
pub fn status() -> Status {
    match run(&["version", "--format", "{{.Server.Version}}"], QUERY) {
        Ok(v) => Status { installed: true, running: true, version: v.trim().to_string(), error: String::new() },
        Err(e) => {
            let msg = e.to_string();
            let installed = msg != "docker is not installed";
            Status { installed, running: false, version: String::new(), error: msg }
        }
    }
}

/// `docker ... --format json` prints one object per line (older versions print a JSON array instead).
fn json_lines(text: &str) -> Vec<Value> {
    let t = text.trim();
    if t.starts_with('[') {
        return serde_json::from_str::<Vec<Value>>(t).unwrap_or_default();
    }
    t.lines().filter_map(|l| serde_json::from_str(l.trim()).ok()).collect()
}

fn text(v: &Value, k: &str) -> String {
    v[k].as_str().unwrap_or_default().to_string()
}

/// "0.0.0.0:6379->6379/tcp, [::]:6379->6379/tcp, 80/tcp" -> one entry per distinct mapping.
/// A port range is skipped: it has no single port to open.
pub fn parse_ports(s: &str) -> Vec<Port> {
    let mut out: Vec<Port> = vec![];
    for part in s.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        let (map, proto) = part.rsplit_once('/').unwrap_or((part, "tcp"));
        let (host, container) = match map.split_once("->") {
            Some((h, c)) => (h.rsplit_once(':').map(|(_, p)| p).unwrap_or(h).parse::<u16>().ok(), c),
            None => (None, map),
        };
        let Ok(container) = container.parse::<u16>() else { continue };
        if map.contains("->") && host.is_none() {
            continue;
        }
        let p = Port { host, container, proto: proto.to_string() };
        if !out.contains(&p) {
            out.push(p);
        }
    }
    out
}

/// The `Labels` field is `k=v,k=v`; values here (paths on Windows) never contain a comma in practice.
fn label<'a>(labels: &'a str, key: &str) -> &'a str {
    labels.split(',').find_map(|kv| kv.strip_prefix(key).and_then(|r| r.strip_prefix('='))).unwrap_or("")
}

pub fn parse_containers(out: &str) -> Vec<Container> {
    json_lines(out)
        .iter()
        .map(|c| {
            let labels = c["Labels"].as_str().unwrap_or_default();
            Container {
                id: text(c, "ID"),
                name: text(c, "Names").split(',').next().unwrap_or("").to_string(),
                image: text(c, "Image"),
                state: text(c, "State"),
                status: text(c, "Status"),
                ports: parse_ports(c["Ports"].as_str().unwrap_or_default()),
                created: text(c, "CreatedAt"),
                project: label(labels, "com.docker.compose.project").to_string(),
                working_dir: label(labels, "com.docker.compose.project.working_dir").to_string(),
                service: label(labels, "com.docker.compose.service").to_string(),
                // Mounts mixes volume names and bind paths; a path never passes the name check
                volumes: c["Mounts"].as_str().unwrap_or_default().split(',').filter(|m| valid_name(m)).map(String::from).collect(),
            }
        })
        .collect()
}

fn pct(s: &str) -> f64 {
    s.trim().trim_end_matches('%').parse().unwrap_or(0.0)
}

pub fn parse_stats(out: &str) -> Vec<Stat> {
    json_lines(out)
        .iter()
        .map(|s| Stat {
            id: text(s, "ID"),
            name: text(s, "Name"),
            cpu: pct(&text(s, "CPUPerc")),
            mem: text(s, "MemUsage"),
            mem_pct: pct(&text(s, "MemPerc")),
        })
        .collect()
}

/// One row per image id; every repository:tag of it is a tag.
pub fn parse_images(out: &str) -> Vec<Image> {
    let mut v: Vec<Image> = vec![];
    for i in json_lines(out) {
        let (repo, tag) = (text(&i, "Repository"), text(&i, "Tag"));
        let tag = if repo == "<none>" { String::new() } else { format!("{repo}:{tag}") };
        let id = text(&i, "ID");
        if let Some(e) = v.iter_mut().find(|e| e.id == id) {
            if !tag.is_empty() {
                e.tags.push(tag);
            }
            continue;
        }
        v.push(Image {
            id,
            tags: if tag.is_empty() { vec![] } else { vec![tag] },
            size: text(&i, "Size"),
            created: text(&i, "CreatedAt"),
            containers: text(&i, "Containers").parse().unwrap_or(0),
        });
    }
    v
}

pub fn parse_volumes(out: &str, containers: &[Container]) -> Vec<Volume> {
    json_lines(out)
        .iter()
        .map(|v| {
            let name = text(v, "Name");
            let used_by = containers.iter().filter(|c| c.volumes.contains(&name)).map(|c| c.name.clone()).collect();
            Volume { name, driver: text(v, "Driver"), mountpoint: text(v, "Mountpoint"), used_by }
        })
        .collect()
}

/// Compose projects seen in the containers (label `com.docker.compose.project`), by name.
pub fn group_projects(containers: &[Container]) -> Vec<Project> {
    let mut out: Vec<Project> = vec![];
    for c in containers.iter().filter(|c| !c.project.is_empty()) {
        let i = match out.iter().position(|p| p.name == c.project) {
            Some(i) => i,
            None => {
                out.push(Project { name: c.project.clone(), working_dir: c.working_dir.clone(), running: 0, total: 0 });
                out.len() - 1
            }
        };
        out[i].total += 1;
        out[i].running += usize::from(c.state == "running");
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

pub fn containers() -> Result<Vec<Container>> {
    // --no-trunc: the Mounts column is cut at 15 characters otherwise
    Ok(parse_containers(&run(&["ps", "-a", "--no-trunc", "--format", "json"], QUERY)?))
}

pub fn stats() -> Result<Vec<Stat>> {
    Ok(parse_stats(&run(&["stats", "--no-stream", "--format", "json"], QUERY)?))
}

pub fn images() -> Result<Vec<Image>> {
    Ok(parse_images(&run(&["images", "--format", "json"], QUERY)?))
}

pub fn volumes() -> Result<Vec<Volume>> {
    let list = containers()?;
    Ok(parse_volumes(&run(&["volume", "ls", "--format", "json"], QUERY)?, &list))
}

/// The compose file of a worktree, if it has one (the first of the names Compose itself looks for).
pub fn compose_file(wt: &Path) -> Option<&'static str> {
    COMPOSE_FILES.iter().copied().find(|f| wt.join(f).is_file())
}

/// `axelstra-<worktree folder>`, cleaned to what a compose project name accepts (lowercase letters, digits, - and _).
pub fn compose_project(wt: &Path) -> String {
    let base = wt.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    let clean: String = base.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '-' }).collect();
    format!("axelstra-{}", clean.trim_matches('-'))
}

/// One action on one thing: `action` is `container.start`, `image.remove`, `prune.volumes`, `compose.up`...
/// `target` is the container, image or volume; `wt` the (already checked) worktree for compose.
pub fn action(action: &str, target: &str, force: bool, volumes: bool, wt: Option<&Path>) -> Result<String> {
    let out = match action {
        "container.start" | "container.stop" | "container.restart" | "container.pause" | "container.unpause" | "container.kill" => {
            let verb = &action["container.".len()..];
            run(&[verb, name_arg("container", target)?], ACTION)?
        }
        "container.remove" => {
            let mut a = vec!["rm"];
            if force {
                a.push("-f");
            }
            if volumes {
                a.push("-v");
            }
            a.push(name_arg("container", target)?);
            run(&a, ACTION)?
        }
        "image.remove" => {
            if !valid_reference(target) {
                bail!("invalid image reference {target:?}");
            }
            run(&if force { vec!["rmi", "-f", target] } else { vec!["rmi", target] }, ACTION)?
        }
        "volume.remove" => run(&["volume", "rm", name_arg("volume", target)?], ACTION)?,
        "prune.containers" => run(&["container", "prune", "-f"], ACTION)?,
        "prune.images" => run(&["image", "prune", "-f"], ACTION)?,
        // anonymous volumes nobody uses; named ones are only removed one by one, on purpose
        "prune.volumes" => run(&["volume", "prune", "-f"], ACTION)?,
        "compose.up" | "compose.down" => {
            let wt = wt.context("compose needs a worktree")?;
            let file = compose_file(wt).context("this worktree has no compose file")?;
            let project = compose_project(wt);
            let mut a = vec!["compose", "-p", project.as_str(), "-f", file];
            if action == "compose.up" {
                a.extend(["up", "-d"]);
            } else {
                a.push("down");
            }
            // compose reads the file relative to the working dir, so run it there
            let o = git::quiet(&mut Command::new("docker")).current_dir(wt).args(&a).stdin(Stdio::null()).output().context("could not start docker")?;
            if !o.status.success() {
                bail!("{}", String::from_utf8_lossy(&o.stderr).trim());
            }
            // compose writes its progress to stderr
            String::from_utf8_lossy(&o.stderr).into_owned()
        }
        _ => bail!("unknown docker action {action}"),
    };
    Ok(out.trim().to_string())
}

/// The `docker.*` methods that only look or act; `store` is where registered projects live.
/// `docker.containers` also answers for the worktree's own compose stack when it is given one.
pub fn call(store: &Path, method: &str, p: &Value) -> Result<Value> {
    let worktree = |p: &Value| crate::projects::known_worktree(store, p["worktree"].as_str().unwrap_or_default());
    Ok(match method {
        "docker.status" => serde_json::to_value(status())?,
        "docker.containers" => {
            let list = containers()?;
            let compose = match p["worktree"].as_str() {
                Some(_) => {
                    let wt = worktree(p)?;
                    let project = compose_project(&wt);
                    let services: Vec<&Container> = list.iter().filter(|c| c.project == project).collect();
                    json!({"file": compose_file(&wt), "project": project, "services": services})
                }
                None => Value::Null,
            };
            json!({"projects": group_projects(&list), "containers": list, "compose": compose})
        }
        "docker.stats" => json!({"stats": stats()?}),
        "docker.images" => json!({"images": images()?}),
        "docker.volumes" => json!({"volumes": volumes()?}),
        "docker.action" => {
            let a = p["action"].as_str().ok_or_else(|| anyhow::anyhow!("missing action"))?;
            let wt = if a.starts_with("compose.") { Some(worktree(p)?) } else { None };
            let output = action(a, p["id"].as_str().unwrap_or_default(), p["force"].as_bool().unwrap_or(false), p["volumes"].as_bool().unwrap_or(false), wt.as_deref())?;
            json!({"output": output})
        }
        _ => bail!("unknown method {method}"),
    })
}

/// What `docker.open_logs` / `docker.open_shell` start: a normal terminal session with a fixed command, in the worktree.
pub fn terminal(store: &Path, method: &str, p: &Value) -> Result<(Spawn, String, Value)> {
    let wt = crate::projects::known_worktree(store, p["worktree"].as_str().unwrap_or_default())?;
    let id = p["id"].as_str().unwrap_or_default();
    let (mut argv, label) = if method == "docker.open_logs" { (logs_argv(id)?, format!("logs {id}")) } else { (shell_argv(id)?, format!("shell {id}")) };
    let program = argv.remove(0);
    let spawn = Spawn { program, args: argv, cwd: Some(wt.clone()), env: vec![], cols: 120, rows: 30 };
    let meta = json!({"project": p["project"], "worktree": wt.to_string_lossy(), "kind": "docker"});
    Ok((spawn, label, meta))
}

/// argv of the session behind the Logs button
pub fn logs_argv(id: &str) -> Result<Vec<String>> {
    let id = name_arg("container", id)?;
    Ok(["docker", "logs", "-f", "--tail", "500", id].map(String::from).to_vec())
}

/// argv of the session behind the Shell button
pub fn shell_argv(id: &str) -> Result<Vec<String>> {
    let id = name_arg("container", id)?;
    Ok(["docker", "exec", "-it", id, "sh", "-c", SHELL_SCRIPT].map(String::from).to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    const PS: &str = concat!(
        r#"{"CreatedAt":"2026-10-18 11:09:52 +0100 BST","ID":"980f3b463531","Image":"postgres:18-alpine","Labels":"com.docker.compose.project.working_dir=C:\\Users\\a\\wt,com.docker.compose.project=demo,com.docker.compose.service=postgres","Mounts":"demo_pgdata,C:\\Users\\a\\wt","Names":"demo-postgres-1","Ports":"0.0.0.0:5432->5432/tcp, [::]:5432->5432/tcp","State":"running","Status":"Up 2 hours"}"#,
        "\n",
        r#"{"CreatedAt":"2026-10-18 11:09:53 +0100 BST","ID":"522a5da574ef","Image":"redis:8-alpine","Labels":"com.docker.compose.project=demo","Mounts":"","Names":"demo-redis-1","Ports":"6379/tcp","State":"exited","Status":"Exited (0) 4 days ago"}"#,
        "\n",
        r#"{"CreatedAt":"2026-10-18 11:09:54 +0100 BST","ID":"aaaaaaaaaaaa","Image":"nginx","Labels":"","Mounts":"","Names":"web","Ports":"","State":"running","Status":"Up 1 minute"}"#,
        "\n"
    );

    #[test]
    fn names_and_references() {
        for ok in ["web", "demo-postgres-1", "a.b_c", "980f3b463531", "9"] {
            assert!(valid_name(ok), "{ok}");
        }
        for bad in ["", "-rf", ".x", "a b", "a;b", "a$(x)", "a/b", "a\nb", "é", "--all", "a&b"] {
            assert!(!valid_name(bad), "{bad:?}");
        }
        for ok in ["redis", "redis:8-alpine", "ghcr.io/org/app:1.2", "app@sha256:abc123", "721873c34ceb"] {
            assert!(valid_reference(ok), "{ok}");
        }
        for bad in ["", "-f", "a b", "a;b", ":tag", "/x", "a|b", "a`b`"] {
            assert!(!valid_reference(bad), "{bad:?}");
        }
        assert!(!valid_name(&"a".repeat(256)));
    }

    #[test]
    fn actions_refuse_bad_targets_before_running_anything() {
        for a in ["container.start", "container.remove", "volume.remove"] {
            assert!(action(a, "-f; rm", false, false, None).unwrap_err().to_string().contains("invalid"), "{a}");
        }
        assert!(action("image.remove", "a b", false, false, None).is_err());
        assert!(action("container.explode", "web", false, false, None).unwrap_err().to_string().contains("unknown"));
        assert!(action("compose.up", "", false, false, None).is_err());
        assert!(logs_argv("x y").is_err() && shell_argv("--privileged").is_err());
    }

    #[test]
    fn terminal_argv_are_fixed() {
        assert_eq!(logs_argv("web").unwrap(), ["docker", "logs", "-f", "--tail", "500", "web"]);
        let sh = shell_argv("web").unwrap();
        assert_eq!(&sh[..6], ["docker", "exec", "-it", "web", "sh", "-c"]);
        assert!(sh[6].contains("exec bash") && sh[6].contains("exec sh"));
    }

    #[test]
    fn parses_ps_lines() {
        let v = parse_containers(PS);
        assert_eq!(v.len(), 3);
        assert_eq!((v[0].name.as_str(), v[0].image.as_str(), v[0].state.as_str()), ("demo-postgres-1", "postgres:18-alpine", "running"));
        assert_eq!((v[0].project.as_str(), v[0].service.as_str(), v[0].working_dir.as_str()), ("demo", "postgres", r"C:\Users\a\wt"));
        assert_eq!(v[0].ports, vec![Port { host: Some(5432), container: 5432, proto: "tcp".into() }]);
        assert_eq!(v[0].volumes, vec!["demo_pgdata"]);
        assert_eq!(v[1].ports, vec![Port { host: None, container: 6379, proto: "tcp".into() }]);
        assert_eq!(v[2].project, "");
        assert!(parse_containers("").is_empty());
        assert!(parse_containers("not json\n").is_empty());
        // an older docker prints an array
        assert_eq!(parse_containers(&format!("[{}]", PS.lines().next().unwrap())).len(), 1);
    }

    #[test]
    fn parses_ports() {
        assert_eq!(parse_ports("").len(), 0);
        let p = parse_ports("0.0.0.0:8080->80/tcp, 0.0.0.0:53->53/udp, 9000/tcp, 0.0.0.0:8000-8002->8000-8002/tcp");
        assert_eq!(p.len(), 3);
        assert_eq!((p[0].host, p[0].container, p[0].proto.as_str()), (Some(8080), 80, "tcp"));
        assert_eq!((p[1].host, p[1].proto.as_str()), (Some(53), "udp"));
        assert_eq!((p[2].host, p[2].container), (None, 9000));
    }

    #[test]
    fn groups_compose_projects() {
        let v = group_projects(&parse_containers(PS));
        assert_eq!(v.len(), 1);
        assert_eq!((v[0].name.as_str(), v[0].running, v[0].total), ("demo", 1, 2));
        assert_eq!(v[0].working_dir, r"C:\Users\a\wt");
    }

    #[test]
    fn parses_stats() {
        let s = parse_stats(r#"{"CPUPerc":"0.15%","ID":"85fb26f106bf","MemPerc":"0.08%","MemUsage":"11.78MiB / 15.2GiB","Name":"redis-1"}"#);
        assert_eq!(s, vec![Stat { id: "85fb26f106bf".into(), name: "redis-1".into(), cpu: 0.15, mem: "11.78MiB / 15.2GiB".into(), mem_pct: 0.08 }]);
        assert_eq!(parse_stats(r#"{"CPUPerc":"--","ID":"x","MemPerc":"--","MemUsage":"0B / 0B","Name":"n"}"#)[0].cpu, 0.0);
    }

    #[test]
    fn parses_images_merging_tags_of_one_id() {
        let out = concat!(
            r#"{"Containers":"7","CreatedAt":"2026-09-21","ID":"3811787313eb","Repository":"redis","Size":"161MB","Tag":"8-alpine"}"#, "\n",
            r#"{"Containers":"7","CreatedAt":"2026-09-21","ID":"3811787313eb","Repository":"redis","Size":"161MB","Tag":"8"}"#, "\n",
            r#"{"Containers":"0","CreatedAt":"2026-09-17","ID":"721873c34ceb","Repository":"<none>","Size":"420MB","Tag":"<none>"}"#
        );
        let v = parse_images(out);
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].tags, vec!["redis:8-alpine", "redis:8"]);
        assert_eq!((v[0].containers, v[0].size.as_str()), (7, "161MB"));
        assert!(v[1].tags.is_empty());
    }

    #[test]
    fn parses_volumes_with_their_users() {
        let c = parse_containers(PS);
        let out = concat!(
            r#"{"Driver":"local","Mountpoint":"/var/lib/docker/volumes/demo_pgdata/_data","Name":"demo_pgdata"}"#, "\n",
            r#"{"Driver":"local","Mountpoint":"/x","Name":"orphan"}"#
        );
        let v = parse_volumes(out, &c);
        assert_eq!(v[0].used_by, vec!["demo-postgres-1"]);
        assert!(v[1].used_by.is_empty());
    }

    #[test]
    fn detects_compose_files_and_names_the_project() {
        let dir = std::env::temp_dir().join(format!("axelstra-docker-{}", std::process::id())).join("My Feature.x");
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(compose_file(&dir), None);
        std::fs::write(dir.join("docker-compose.yaml"), "services: {}").unwrap();
        assert_eq!(compose_file(&dir), Some("docker-compose.yaml"));
        std::fs::write(dir.join("compose.yml"), "services: {}").unwrap();
        assert_eq!(compose_file(&dir), Some("compose.yml"));
        assert_eq!(compose_project(&dir), "axelstra-my-feature-x");
        assert!(valid_name(&compose_project(&dir)));
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
    }
}
