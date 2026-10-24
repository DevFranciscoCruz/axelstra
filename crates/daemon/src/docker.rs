//! A small Docker Desktop on top of the `docker` CLI. Every call is a fixed argument list (never a shell), has a timeout,
//! and every id, name and image reference is checked before it reaches the command line.

use anyhow::{bail, Context, Result};
use axelstra_core::pty::Spawn;
use serde::Serialize;
use serde_json::{json, Value};
use std::io::Read;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

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
    pub networks: Vec<String>,
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
    /// no repository or tag: left behind by a rebuild
    pub dangling: bool,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct Volume {
    pub name: String,
    pub driver: String,
    pub mountpoint: String,
    pub used_by: Vec<String>,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct Network {
    pub id: String,
    pub name: String,
    pub driver: String,
    pub scope: String,
    pub containers: Vec<String>,
}

#[derive(Serialize, Debug, PartialEq, Default)]
pub struct Inspect {
    pub command: String,
    pub workdir: String,
    pub restart: String,
    pub env: Vec<(String, String)>,
    /// (kind, source, destination, writable)
    pub mounts: Vec<(String, String, String, bool)>,
    /// (network, ip address)
    pub networks: Vec<(String, String)>,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct FsEntry {
    pub name: String,
    pub dir: bool,
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
    s.len() <= 255
        && c.next().is_some_and(|f| f.is_ascii_alphanumeric())
        && c.all(|x| x.is_ascii_alphanumeric() || matches!(x, '_' | '.' | '-'))
}

/// An image reference (`repo/name:tag`, `name@sha256:...`) or id: the name charset plus `/`, `:` and `@`.
pub fn valid_reference(s: &str) -> bool {
    let mut c = s.chars();
    s.len() <= 255
        && c.next().is_some_and(|f| f.is_ascii_alphanumeric())
        && c.all(|x| x.is_ascii_alphanumeric() || matches!(x, '_' | '.' | '-' | '/' | ':' | '@'))
}

fn name_arg<'a>(kind: &str, s: &'a str) -> Result<&'a str> {
    if !valid_name(s) {
        bail!("invalid {kind} {s:?}: only letters, digits, _ . - are allowed");
    }
    Ok(s)
}

/// Runs docker; stdout on success, the trimmed stderr as the error. `NotFound` means docker is not installed.
fn run(args: &[&str], limit: Duration) -> Result<String> {
    run_full(args, limit).map(|(out, _)| out)
}

/// Most of each stream kept from one docker call (`docker logs` of a chatty container, a huge `inspect`).
const MAX_OUT: usize = 8 << 20;

/// `docker`, started through the shared runner: a timeout kills the whole tree, output is capped, a lingering
/// grandchild holding the pipe cannot hang the caller.
fn docker_capped(cmd: &mut Command, limit: Duration, what: &str) -> Result<axelstra_core::exec::Captured> {
    let o = axelstra_core::exec::run_capped(cmd, limit, MAX_OUT).map_err(|e| match e.downcast_ref::<std::io::Error>() {
        Some(io) if io.kind() == std::io::ErrorKind::NotFound => anyhow::anyhow!("docker is not installed"),
        _ => e.context("could not start docker"),
    })?;
    if o.timed_out {
        bail!("docker {what} timed out after {}s", limit.as_secs());
    }
    Ok(o)
}

/// Same, keeping stderr too (`docker logs` prints the container's stderr there).
fn run_full(args: &[&str], limit: Duration) -> Result<(String, String)> {
    let what = args.first().copied().unwrap_or("");
    let o = docker_capped(Command::new("docker").args(args), limit, what)?;
    if !o.success() {
        let msg = o.stderr_lossy();
        let msg = msg.trim();
        bail!("{}", if msg.is_empty() { format!("docker {what} failed") } else { msg.to_string() });
    }
    Ok((o.stdout_lossy(), o.stderr_lossy()))
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
                networks: c["Networks"].as_str().unwrap_or_default().split(',').filter(|m| valid_name(m)).map(String::from).collect(),
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
            dangling: repo == "<none>",
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

pub fn parse_networks(out: &str, containers: &[Container]) -> Vec<Network> {
    json_lines(out)
        .iter()
        .map(|n| {
            let name = text(n, "Name");
            let on = containers.iter().filter(|c| c.networks.contains(&name)).map(|c| c.name.clone()).collect();
            Network { id: text(n, "ID"), name, driver: text(n, "Driver"), scope: text(n, "Scope"), containers: on }
        })
        .collect()
}

/// The parts of `docker inspect` the detail pane shows.
pub fn parse_inspect(out: &str) -> Result<Inspect> {
    let v: Value = serde_json::from_str(out).context("docker inspect returned invalid JSON")?;
    let c = v.get(0).context("docker inspect returned nothing")?;
    fn words(a: &Value) -> Vec<&str> {
        a.as_array().map(|l| l.iter().filter_map(Value::as_str).collect()).unwrap_or_default()
    }
    let mut cmd = vec![c["Path"].as_str().unwrap_or_default()];
    cmd.extend(words(&c["Args"]));
    let mut env: Vec<(String, String)> = words(&c["Config"]["Env"])
        .iter()
        .map(|e| e.split_once('=').map(|(k, v)| (k.to_string(), v.to_string())).unwrap_or((e.to_string(), String::new())))
        .collect();
    env.sort();
    let mounts = c["Mounts"]
        .as_array()
        .map(|l| {
            l.iter()
                .map(|m| {
                    let src = if m["Type"] == "volume" { m["Name"].as_str() } else { m["Source"].as_str() };
                    (text(m, "Type"), src.unwrap_or_default().to_string(), text(m, "Destination"), m["RW"].as_bool().unwrap_or(true))
                })
                .collect()
        })
        .unwrap_or_default();
    let networks = c["NetworkSettings"]["Networks"]
        .as_object()
        .map(|o| o.iter().map(|(k, n)| (k.clone(), text(n, "IPAddress"))).collect())
        .unwrap_or_default();
    Ok(Inspect {
        command: cmd.join(" ").trim().to_string(),
        workdir: text(&c["Config"], "WorkingDir"),
        restart: text(&c["HostConfig"]["RestartPolicy"], "Name"),
        env,
        mounts,
        networks,
    })
}

/// `ls -1Ap` lines: a trailing slash marks a directory. Folders first, then by name.
pub fn parse_ls(out: &str) -> Vec<FsEntry> {
    let mut v: Vec<FsEntry> = out
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| match l.strip_suffix('/') {
            Some(n) => FsEntry { name: n.to_string(), dir: true },
            None => FsEntry { name: l.to_string(), dir: false },
        })
        .collect();
    v.sort_by(|a, b| b.dir.cmp(&a.dir).then(a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    v
}

/// A path inside a container: absolute, no control characters (it is one argument, never parsed by a shell).
pub fn valid_container_path(p: &str) -> bool {
    p.starts_with('/') && p.len() <= 4096 && !p.chars().any(char::is_control)
}

fn path_arg(p: &str) -> Result<&str> {
    if !valid_container_path(p) {
        bail!("invalid path {p:?}: it must be absolute");
    }
    Ok(p)
}

pub fn networks() -> Result<Vec<Network>> {
    let list = containers()?;
    Ok(parse_networks(&run(&["network", "ls", "--no-trunc", "--format", "json"], QUERY)?, &list))
}

pub fn inspect(id: &str) -> Result<Inspect> {
    parse_inspect(&run(&["inspect", "--type", "container", name_arg("container", id)?], QUERY)?)
}

/// Last lines of a container's log, stdout and stderr together.
pub fn logs(id: &str, tail: u32) -> Result<String> {
    let tail = tail.clamp(1, 2000).to_string();
    let (out, err) = run_full(&["logs", "--tail", &tail, name_arg("container", id)?], QUERY)?;
    Ok(if err.is_empty() { out } else { format!("{out}{err}") })
}

pub fn fs_list(id: &str, path: &str) -> Result<Vec<FsEntry>> {
    Ok(parse_ls(&run(&["exec", name_arg("container", id)?, "ls", "-1Ap", "--", path_arg(path)?], QUERY)?))
}

const READ_MAX: u64 = 512 * 1024;

/// Copies one file out with `docker cp` into a throwaway folder, reads it as text and deletes the copy.
pub fn fs_read(id: &str, path: &str) -> Result<String> {
    let src = format!("{}:{}", name_arg("container", id)?, path_arg(path)?);
    let mut r = [0u8; 8];
    getrandom::getrandom(&mut r).map_err(|e| anyhow::anyhow!("no randomness: {e}"))?;
    let dir = std::env::temp_dir().join(format!("axelstra-docker-{}", r.iter().map(|b| format!("{b:02x}")).collect::<String>()));
    std::fs::create_dir_all(&dir)?;
    let result = (|| {
        run(&["cp", &src, &dir.to_string_lossy()], QUERY)?;
        let file = std::fs::read_dir(&dir)?.flatten().next().context("nothing was copied")?.path();
        if !file.is_file() {
            bail!("that is a folder, not a file");
        }
        let mut bytes = Vec::new();
        std::fs::File::open(&file)?.take(READ_MAX).read_to_end(&mut bytes)?;
        if bytes.contains(&0) {
            bail!("binary file, not shown");
        }
        let truncated = std::fs::metadata(&file)?.len() > READ_MAX;
        Ok(format!("{}{}", String::from_utf8_lossy(&bytes), if truncated { "\n… (cut at 512 KB)" } else { "" }))
    })();
    let _ = std::fs::remove_dir_all(&dir);
    result
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

/// Reads the resolved `docker compose config --format json` and returns `(refusals, warnings)`: what must not be
/// started from a worktree (privileged containers, the engine socket, bind mounts outside the worktree, host
/// namespaces) and what only deserves a look (added capabilities, devices, security options).
pub fn compose_audit(config: &str, wt: &Path) -> (Vec<String>, Vec<String>) {
    let (mut deny, mut warn) = (vec![], vec![]);
    let Ok(v) = serde_json::from_str::<Value>(config) else {
        return (vec!["could not read the resolved compose config".into()], warn);
    };
    let wt_s = axelstra_core::paths::norm_path(wt);
    let Some(services) = v["services"].as_object() else { return (deny, warn) };
    for (name, svc) in services {
        if svc["privileged"].as_bool() == Some(true) {
            deny.push(format!("service {name} is privileged"));
        }
        for key in ["network_mode", "pid", "ipc", "userns_mode", "uts"] {
            if svc[key].as_str() == Some("host") {
                deny.push(format!("service {name} uses the host namespace ({key}: host)"));
            }
        }
        for m in svc["volumes"].as_array().into_iter().flatten() {
            // long syntax after `config`; a bare string only on very old compose versions
            let (kind, source) = match m {
                Value::Object(_) => (m["type"].as_str().unwrap_or("volume"), m["source"].as_str().unwrap_or_default().to_string()),
                Value::String(s) => ("bind", s.split(':').next().unwrap_or_default().to_string()),
                _ => continue,
            };
            let lower = source.to_lowercase();
            if lower.contains("docker.sock") || lower.contains("docker_engine") {
                deny.push(format!("service {name} mounts the Docker engine socket ({source})"));
            } else if kind == "bind" && !axelstra_core::paths::is_within(&source, &wt_s) {
                deny.push(format!("service {name} bind-mounts {source}, outside the worktree"));
            }
        }
        for d in svc["devices"].as_array().into_iter().flatten() {
            warn.push(format!("service {name} gets device {}", d.as_str().or_else(|| d["source"].as_str()).unwrap_or("?")));
        }
        let caps: Vec<&str> = svc["cap_add"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
        if !caps.is_empty() {
            warn.push(format!("service {name} adds capabilities {}", caps.join(", ")));
        }
        for o in svc["security_opt"].as_array().into_iter().flatten().filter_map(Value::as_str) {
            warn.push(format!("service {name} sets security_opt {o}"));
        }
    }
    (deny, warn)
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
            let compose = |tail: &[&str], limit: Duration| -> Result<axelstra_core::exec::Captured> {
                let mut c = Command::new("docker");
                c.current_dir(wt).args(["compose", "-p", project.as_str(), "-f", file]).args(tail);
                docker_capped(&mut c, limit, "compose")
            };
            // what compose would actually run, with includes, extends and variables resolved: a worktree's compose
            // file is untrusted (an agent or a PR wrote it), and `up` can mount the host or take the engine over
            let (refusals, mut warnings) = match compose(&["config", "--format", "json"], ACTION) {
                Ok(o) if o.success() => compose_audit(&o.stdout_lossy(), wt),
                Ok(o) => (vec![format!("`docker compose config` failed: {}", o.stderr_lossy().trim())], vec![]),
                Err(e) => (vec![e.to_string()], vec![]),
            };
            if action == "compose.up" && !refusals.is_empty() {
                bail!("refusing compose up:
- {}", refusals.join("
- "));
            }
            // down only stops what exists, so a finding there is reported, not blocking
            warnings.extend(refusals);
            let o = compose(if action == "compose.up" { &["up", "-d"] } else { &["down"] }, ACTION * 5)?;
            if !o.success() {
                bail!("{}", o.stderr_lossy().trim());
            }
            // compose writes its progress to stderr
            let mut text = o.stderr_lossy();
            if !warnings.is_empty() {
                text = format!("warnings:
- {}
{text}", warnings.join("
- "));
            }
            text
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
        "docker.networks" => json!({"networks": networks()?}),
        "docker.inspect" => serde_json::to_value(inspect(p["id"].as_str().unwrap_or_default())?)?,
        "docker.logs" => json!({"text": logs(p["id"].as_str().unwrap_or_default(), p["tail"].as_u64().unwrap_or(500) as u32)?}),
        "docker.fs_list" => json!({"entries": fs_list(p["id"].as_str().unwrap_or_default(), p["path"].as_str().unwrap_or("/"))?}),
        "docker.fs_read" => json!({"text": fs_read(p["id"].as_str().unwrap_or_default(), p["path"].as_str().unwrap_or_default())?}),
        "docker.images" => json!({"images": images()?}),
        "docker.volumes" => json!({"volumes": volumes()?}),
        "docker.action" => {
            let a = p["action"].as_str().ok_or_else(|| anyhow::anyhow!("missing action"))?;
            let wt = if a.starts_with("compose.") { Some(worktree(p)?) } else { None };
            let output = action(
                a,
                p["id"].as_str().unwrap_or_default(),
                p["force"].as_bool().unwrap_or(false),
                p["volumes"].as_bool().unwrap_or(false),
                wt.as_deref(),
            )?;
            json!({"output": output})
        }
        _ => bail!("unknown method {method}"),
    })
}

/// What `docker.open_logs` / `docker.open_shell` start: a normal terminal session with a fixed command, in the worktree.
pub fn terminal(store: &Path, method: &str, p: &Value) -> Result<(Spawn, String, Value)> {
    let wt = crate::projects::known_worktree(store, p["worktree"].as_str().unwrap_or_default())?;
    let id = p["id"].as_str().unwrap_or_default();
    let (mut argv, label) =
        if method == "docker.open_logs" { (logs_argv(id)?, format!("logs {id}")) } else { (shell_argv(id)?, format!("shell {id}")) };
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
        let s =
            parse_stats(r#"{"CPUPerc":"0.15%","ID":"85fb26f106bf","MemPerc":"0.08%","MemUsage":"11.78MiB / 15.2GiB","Name":"redis-1"}"#);
        assert_eq!(
            s,
            vec![Stat { id: "85fb26f106bf".into(), name: "redis-1".into(), cpu: 0.15, mem: "11.78MiB / 15.2GiB".into(), mem_pct: 0.08 }]
        );
        assert_eq!(parse_stats(r#"{"CPUPerc":"--","ID":"x","MemPerc":"--","MemUsage":"0B / 0B","Name":"n"}"#)[0].cpu, 0.0);
    }

    #[test]
    fn parses_images_merging_tags_of_one_id() {
        let out = concat!(
            r#"{"Containers":"7","CreatedAt":"2026-09-21","ID":"3811787313eb","Repository":"redis","Size":"161MB","Tag":"8-alpine"}"#,
            "\n",
            r#"{"Containers":"7","CreatedAt":"2026-09-21","ID":"3811787313eb","Repository":"redis","Size":"161MB","Tag":"8"}"#,
            "\n",
            r#"{"Containers":"0","CreatedAt":"2026-09-17","ID":"721873c34ceb","Repository":"<none>","Size":"420MB","Tag":"<none>"}"#
        );
        let v = parse_images(out);
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].tags, vec!["redis:8-alpine", "redis:8"]);
        assert_eq!((v[0].containers, v[0].size.as_str()), (7, "161MB"));
        assert!(v[1].tags.is_empty() && v[1].dangling && !v[0].dangling);
    }

    #[test]
    fn parses_networks_with_their_containers() {
        let mut c = parse_containers(PS);
        c[0].networks = vec!["demo_default".into()];
        let out = r#"{"Driver":"bridge","ID":"abc","Name":"demo_default","Scope":"local"}"#;
        let n = parse_networks(out, &c);
        assert_eq!(
            (n[0].name.as_str(), n[0].driver.as_str(), n[0].containers.clone()),
            ("demo_default", "bridge", vec!["demo-postgres-1".to_string()])
        );
    }

    #[test]
    fn parses_inspect() {
        let j = r#"[{"Path":"docker-entrypoint.sh","Args":["postgres","-c","x=1"],"Config":{"Env":["B=2","A=1=1","BARE"],"WorkingDir":"/app"},
            "HostConfig":{"RestartPolicy":{"Name":"always"}},
            "Mounts":[{"Type":"volume","Name":"pgdata","Source":"/var/lib/docker/volumes/pgdata/_data","Destination":"/data","RW":true},{"Type":"bind","Source":"/host","Destination":"/h","RW":false}],
            "NetworkSettings":{"Networks":{"demo_default":{"IPAddress":"172.18.0.2"}}}}]"#;
        let i = parse_inspect(j).unwrap();
        assert_eq!(i.command, "docker-entrypoint.sh postgres -c x=1");
        assert_eq!(i.env, vec![("A".into(), "1=1".into()), ("B".into(), "2".into()), ("BARE".into(), "".into())]);
        assert_eq!(i.mounts[0], ("volume".into(), "pgdata".into(), "/data".into(), true));
        assert!(!i.mounts[1].3);
        assert_eq!(i.networks, vec![("demo_default".to_string(), "172.18.0.2".to_string())]);
        assert_eq!((i.workdir.as_str(), i.restart.as_str()), ("/app", "always"));
        assert!(parse_inspect("[]").is_err() && parse_inspect("x").is_err());
    }

    #[test]
    fn lists_container_files_folders_first() {
        let v = parse_ls("zeta\nbin/\n.env\nApp/\n");
        assert_eq!(
            v.iter().map(|e| (e.name.as_str(), e.dir)).collect::<Vec<_>>(),
            [("App", true), ("bin", true), (".env", false), ("zeta", false)]
        );
    }

    #[test]
    fn container_paths_must_be_absolute_and_plain() {
        assert!(valid_container_path("/") && valid_container_path("/etc/my file.conf"));
        for bad in ["", "etc", "-la", "/a\nb", "/a\0b"] {
            assert!(!valid_container_path(bad), "{bad:?}");
        }
        assert!(fs_list("web", "relative").is_err() && fs_read("-x", "/etc/passwd").is_err());
    }

    #[test]
    fn parses_volumes_with_their_users() {
        let c = parse_containers(PS);
        let out = concat!(
            r#"{"Driver":"local","Mountpoint":"/var/lib/docker/volumes/demo_pgdata/_data","Name":"demo_pgdata"}"#,
            "\n",
            r#"{"Driver":"local","Mountpoint":"/x","Name":"orphan"}"#
        );
        let v = parse_volumes(out, &c);
        assert_eq!(v[0].used_by, vec!["demo-postgres-1"]);
        assert!(v[1].used_by.is_empty());
    }

    #[test]
    fn compose_audit_refuses_host_access_and_warns_on_the_rest() {
        let wt = std::env::temp_dir().join("axl-wt-audit");
        let inside = axelstra_core::paths::norm_path(&wt.join("data"));
        let cfg = json!({"services": {
            "ok": {"volumes": [{"type": "bind", "source": inside, "target": "/d"}, {"type": "volume", "source": "v", "target": "/v"}]},
            "bad": {"privileged": true, "network_mode": "host",
                    "volumes": [{"type": "bind", "source": "/var/run/docker.sock", "target": "/s"},
                                {"type": "bind", "source": "/etc", "target": "/e"}],
                    "cap_add": ["SYS_ADMIN"]}
        }});
        let (deny, warn) = compose_audit(&cfg.to_string(), &wt);
        assert!(deny.iter().all(|d| d.contains("bad")), "{deny:?}");
        for needle in ["privileged", "host namespace", "engine socket", "outside the worktree"] {
            assert!(deny.iter().any(|d| d.contains(needle)), "{needle} in {deny:?}");
        }
        assert!(warn.iter().any(|w| w.contains("SYS_ADMIN")));
        assert_eq!(compose_audit(r#"{"services":{"a":{}}}"#, &wt), (vec![], vec![]));
        assert!(!compose_audit("nope", &wt).0.is_empty());
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

    /// Needs docker and an image that is already present: `cargo test -p axelstra-daemon real_docker -- --ignored --nocapture`.
    /// Only touches a container it creates itself.
    #[test]
    #[ignore]
    fn real_docker() {
        let st = status();
        println!("status: {st:?}");
        if !st.running {
            return;
        }
        let list = containers().unwrap();
        println!("containers: {}, projects: {}", list.len(), group_projects(&list).len());
        println!(
            "stats: {}, images: {}, volumes: {}, networks: {}",
            stats().unwrap().len(),
            images().unwrap().len(),
            volumes().unwrap().len(),
            networks().unwrap().len()
        );
        let image = std::env::var("AXELSTRA_TEST_IMAGE").unwrap_or_else(|_| "redis:8-alpine".into());
        let name = format!("axelstra-test-{}", std::process::id());
        let made = Command::new("docker").args(["run", "-d", "--name", &name, &image]).output().unwrap();
        assert!(made.status.success(), "{}", String::from_utf8_lossy(&made.stderr));
        let r = (|| -> Result<()> {
            println!("inspect: {:?}", inspect(&name)?.command);
            println!("logs: {} bytes", logs(&name, 20)?.len());
            println!("fs_list /: {:?}", fs_list(&name, "/")?.iter().take(5).map(|e| e.name.clone()).collect::<Vec<_>>());
            println!("fs_read /etc/hostname: {:?}", fs_read(&name, "/etc/hostname")?.trim());
            assert!(fs_read(&name, "/etc").is_err());
            action("container.pause", &name, false, false, None)?;
            action("container.unpause", &name, false, false, None)?;
            action("container.restart", &name, false, false, None)?;
            action("container.stop", &name, false, false, None)?;
            action("container.start", &name, false, false, None)?;
            Ok(())
        })();
        let removed = action("container.remove", &name, true, false, None);
        r.unwrap();
        println!("removed: {removed:?}");
        removed.unwrap();
    }
}
