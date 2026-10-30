//! `axelstra dispatch`: what the main session needs to act as a dispatcher. Every request goes to the worktree
//! (topic) it belongs to, or opens a new one. Per-worktree notes (title, requests, archived) live in one JSON file
//! next to the daemon's other state; the app reads them to label worktrees.

use super::DispCmd;
use crate::ctl::{find_project_by_ref, norm, spawn_agent};
use anyhow::{bail, Context, Result};
use axelstra_daemon::client::Client;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

const FOOTER: &str = "

You are the LEAD of this worktree: a dispatcher, not an implementer. You own the topic, but you do not do the work yourself. Run `axelstra skills get axelstration` first. Then: (1) plan the request into subtasks; (2) start one worker per independent subtask in THIS worktree with `axelstra agent . --prompt \"<self-contained task, files, acceptance criteria>\" --report-to $AXELSTRA_SESSION` (workers appear as `Tarefa N` under you; never give two workers the same files, never create more worktrees); (3) do NOT edit code and do NOT run heavy builds or test suites yourself; you only integrate the workers' commits, resolve conflicts and run the final check with `axelstra lock verify -- <cmd>`; (4) wait for their `done`/`ask`/`escalate` reports (answer asks with `axelstra reply`, unblock with `axelstra msg`) and treat reports as claims to verify; (5) when all is integrated and verified, run `axelstra done \"<report: commits, what was verified, what was not verified>\"`: that report goes to whoever dispatched you and your session closes by itself (the worktree is cleaned up once its branch is integrated). Later requests for the same topic are sent to you.";

/// What a dispatched lead starts with: the request, who it is, how it finishes, and the rule against waiting silently.
fn lead_prompt(request: &str) -> String {
    format!("{request}{FOOTER} {} {}", axelstra_core::waiting::NEVER_WAIT_RULE, axelstra_daemon::prompts::BUILD_RULE)
}

/// A lead started by `dispatch` is ephemeral like a `--report-to` worker: it closes itself when it calls `axelstra done`
/// (mode `exit`: never because it went quiet while waiting on its workers). The report goes to the dispatching session.
fn lead_reap() -> Value {
    let mut r = worker_reap();
    r["lead"] = json!(true);
    r
}

/// A direct worker (no lead) is a normal ephemeral worker: mode `exit`, counts against the worker limit, and its
/// `done` report goes to the dispatching session (the project main, $AXELSTRA_SESSION).
fn worker_reap() -> Value {
    let mut r = json!({"mode": "exit"});
    if let Some(p) = std::env::var("AXELSTRA_SESSION").ok().and_then(|s| s.trim().parse::<u32>().ok()) {
        r["parent"] = json!(p);
    }
    r
}

const WORKER_FOOTER: &str = "

You are a WORKER started directly by the project main for this request: there is no lead above you, so you do the work yourself and you integrate it yourself. Steps: (1) implement the request in THIS worktree and commit in small commits; (2) merge the default branch (usually `main`; `git fetch` first when there is a remote) into your branch and resolve any conflicts; (3) run the project's check through the build lock: `axelstra lock verify -- node scripts/check.mjs` (or the check the project documents) and fix what fails; (4) run `axelstra done \"<report: commits, what was verified, what was not verified>\"`: that report goes to whoever dispatched you and your session closes by itself (the worktree is cleaned up once its branch is integrated). Later requests for the same topic start a new worker.";

/// What a direct worker starts with: the request, that it integrates its own work, how it finishes, and the rule
/// against waiting silently.
fn worker_prompt(request: &str) -> String {
    format!("{request}{WORKER_FOOTER} {} {}", axelstra_core::waiting::NEVER_WAIT_RULE, axelstra_daemon::prompts::BUILD_RULE)
}

/// Lead or single worker for a request that did not say. Conservative: only a request that visibly lists two or more
/// independent items asks for a lead (two numbered or bulleted lines, or inline `(1) ... (2) ...` / `1) ... 2) ...`
/// markers); anything ambiguous is one worker.
fn wants_lead(request: &str) -> bool {
    request.lines().filter(|l| is_item_line(l)).count() >= 2 || has_inline_markers(request)
}

/// `1. x`, `2) x`, `(3) x`, `- x`, `* x`, `• x`
fn is_item_line(line: &str) -> bool {
    let t = line.trim_start();
    if let Some(rest) = t.strip_prefix(['-', '*', '\u{2022}']) {
        return rest.starts_with(' ') && !rest.trim().is_empty();
    }
    let t = t.strip_prefix('(').unwrap_or(t);
    let digits = t.chars().take_while(|c| c.is_ascii_digit()).count();
    if digits == 0 || digits > 2 {
        return false;
    }
    t[digits..].strip_prefix(['.', ')']).is_some_and(|r| r.starts_with(' ') && !r.trim().is_empty())
}

/// both a `(1)` and a `(2)` (or a bare `1)` and `2)` that start a word) in the text
fn has_inline_markers(text: &str) -> bool {
    let has = |n: u32| {
        let (paren, bare) = (format!("({n})"), format!("{n})"));
        let before = |i: usize| text[..i].chars().next_back();
        text.match_indices(&paren).any(|(i, _)| before(i).is_none_or(|c| !c.is_alphanumeric()))
            || text.match_indices(&bare).any(|(i, _)| before(i).is_none_or(char::is_whitespace))
    };
    has(1) && has(2)
}

/// `--lead` / `--no-lead` override the heuristic.
fn choose_lead(lead: bool, no_lead: bool, request: &str) -> bool {
    lead || (!no_lead && wants_lead(request))
}

/// Start the agent for a request in `path`: a lead (dispatcher) or one direct worker. Prints which one.
fn start(c: &mut Client, path: &str, agent: &str, request: &str, as_lead: bool) -> Result<u32> {
    if as_lead {
        println!("mode: lead (plans the request and starts workers)");
        spawn_agent(c, Path::new(path), agent, Some(&lead_prompt(request)), Some(lead_reap()), false)
    } else {
        println!("mode: single worker (no lead; it integrates its own work)");
        spawn_agent(c, Path::new(path), agent, Some(&worker_prompt(request)), Some(worker_reap()), false)
    }
}

fn meta_path(c: &mut Client) -> Result<PathBuf> {
    let dir = c.call("app.dir", json!({}))?["dir"].as_str().unwrap_or_default().to_string();
    Ok(PathBuf::from(dir).join("dispatch.json"))
}

fn load(p: &Path) -> Value {
    std::fs::read_to_string(p).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_else(|| json!({}))
}

fn save(p: &Path, v: &Value) -> Result<()> {
    let tmp = p.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(v)?)?;
    std::fs::rename(&tmp, p)?;
    Ok(())
}

fn git(dir: &str, args: &[&str]) -> Option<String> {
    let o = axelstra_core::proc::hidden(Command::new("git").args(args).current_dir(dir)).output().ok()?;
    o.status.success().then(|| String::from_utf8_lossy(&o.stdout).trim().to_string())
}

/// true when GitHub says a pull request from this branch was merged (needs `gh`; any failure counts as no)
fn pr_merged(wt: &str, branch: &str) -> bool {
    git_like("gh", wt, &["pr", "list", "--head", branch, "--state", "merged", "--json", "number", "--limit", "1"])
        .map(|o| o.contains("number"))
        .unwrap_or(false)
}

fn git_like(program: &str, dir: &str, args: &[&str]) -> Option<String> {
    let o = axelstra_core::proc::hidden(Command::new(program).args(args).current_dir(dir)).output().ok()?;
    o.status.success().then(|| String::from_utf8_lossy(&o.stdout).trim().to_string())
}

/// files this worktree changed: committed since the default branch plus uncommitted
fn changed_files(wt: &str, base: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    if let Some(t) = git(wt, &["diff", "--name-only", &format!("{base}...HEAD")]) {
        out.extend(t.lines().map(|l| l.trim().replace('\\', "/")).filter(|l| !l.is_empty()));
    }
    // uncommitted: tracked edits and new files, asked for apart because `git` trims its output, which would eat the
    // leading space of the first `status --porcelain` line and misparse that path
    for args in [&["diff", "--name-only", "HEAD"][..], &["ls-files", "--others", "--exclude-standard"][..]] {
        if let Some(t) = git(wt, args) {
            out.extend(t.lines().map(|l| l.trim().replace('\\', "/")).filter(|l| !l.is_empty()));
        }
    }
    out
}

fn areas(files: &BTreeSet<String>) -> Vec<String> {
    let mut a = BTreeSet::new();
    for f in files {
        let parts: Vec<&str> = f.split('/').collect();
        a.insert(match parts.len() {
            1 => f.clone(),
            2 => format!("{}/", parts[0]),
            _ => format!("{}/{}/", parts[0], parts[1]),
        });
    }
    a.into_iter().take(8).collect()
}

struct Row {
    path: String,
    branch: String,
    title: String,
    areas: Vec<String>,
    files: BTreeSet<String>,
    agents: Vec<u64>,
    merged: bool,
    archived: bool,
    last_request: String,
}

fn is_main_agent(s: &Value) -> bool {
    s["running"] == json!(true) && s["meta"]["kind"] == "agent" && s["meta"]["worker"] != json!(true)
}

fn rows(c: &mut Client, meta: &Value, project: Option<&str>) -> Result<Vec<Row>> {
    let projects = c.call("project.list", json!({}))?;
    let sessions = c.call("session.list", json!({}))?;
    let mut out = Vec::new();
    for p in projects.as_array().cloned().unwrap_or_default() {
        if let Some(r) = project {
            if find_project_by_ref(&json!([p.clone()]), r).is_none() {
                continue;
            }
        }
        let base = p["default_branch"].as_str().unwrap_or("main").to_string();
        for w in p["worktrees"].as_array().cloned().unwrap_or_default() {
            if w["is_main"] == json!(true) {
                continue;
            }
            let path = w["path"].as_str().unwrap_or_default().to_string();
            let branch = w["branch"].as_str().unwrap_or("(detached)").to_string();
            let m = &meta[norm(&path)];
            let files = changed_files(&path, &base);
            let tip = git(&path, &["rev-parse", "HEAD"]);
            let base_tip = git(&path, &["rev-parse", &base]);
            // merged = the branch moved on from where the default branch is now and is contained in it
            let merged = (tip.is_some() && tip != base_tip && git(&path, &["merge-base", "--is-ancestor", "HEAD", &base]).is_some())
                || pr_merged(&path, &branch);
            let agents = sessions
                .as_array()
                .map(|a| a.as_slice())
                .unwrap_or_default()
                .iter()
                .filter(|s| is_main_agent(s) && norm(s["meta"]["worktree"].as_str().unwrap_or_default()) == norm(&path))
                .filter_map(|s| s["id"].as_u64())
                .collect();
            out.push(Row {
                title: m["title"].as_str().unwrap_or(&branch).to_string(),
                areas: areas(&files),
                files,
                agents,
                merged,
                archived: m["archived"] == json!(true),
                last_request: m["requests"].as_array().and_then(|r| r.last()).and_then(|r| r.as_str()).unwrap_or_default().to_string(),
                path,
                branch,
            });
        }
    }
    Ok(out)
}

fn resolve<'a>(rows: &'a [Row], want: &str) -> Result<&'a Row> {
    let w = want.trim();
    let n = norm(w);
    rows.iter()
        .find(|r| norm(&r.path) == n)
        .or_else(|| rows.iter().find(|r| r.branch == w || r.branch == format!("ax/{w}")))
        .or_else(|| rows.iter().find(|r| r.title.eq_ignore_ascii_case(w)))
        .with_context(|| format!("no worktree matches {want:?} (see `axelstra dispatch list`)"))
}

/// Name of the project whose main checkout `want` names (its path, a folder below it, the project's id or name, or the
/// branch checked out there): the main checkout never has tasks, so `send` refuses it.
fn main_checkout_named(projects: &Value, want: &str) -> Option<String> {
    let w = want.trim();
    if let Some(p) = find_project_by_ref(projects, w) {
        return p["name"].as_str().map(String::from);
    }
    let n = norm(w);
    projects.as_array()?.iter().find_map(|p| {
        let main = p["worktrees"].as_array()?.iter().find(|x| x["is_main"] == json!(true))?;
        let path = main["path"].as_str().unwrap_or_default();
        let inside = !n.contains("/.axelstra-wt/") && axelstra_core::paths::is_within(w, path);
        (inside || main["branch"].as_str() == Some(w)).then(|| p["name"].as_str().unwrap_or_default().to_string())
    })
}

fn main_no_tasks(project: String) -> anyhow::Error {
    axelstra_core::coded!(
        "session.main_no_tasks",
        "the main checkout of {project} only takes its main session and terminals: give the task its own worktree with `axelstra dispatch new {project} <name> \"<request>\"`",
        project = project
    )
}

fn entry<'a>(meta: &'a mut Value, path: &str) -> &'a mut Value {
    let e = &mut meta[norm(path)];
    if !e.is_object() {
        *e = json!({});
    }
    e
}

fn remember(meta: &mut Value, path: &str, title: Option<&str>, request: &str) {
    let e = entry(meta, path);
    if let Some(t) = title {
        e["title"] = json!(t);
    } else if e["title"].is_null() {
        e["title"] = json!(request.chars().take(60).collect::<String>());
    }
    let mut reqs: Vec<Value> = e["requests"].as_array().cloned().unwrap_or_default();
    reqs.push(json!(request.chars().take(400).collect::<String>()));
    let keep = reqs.len().saturating_sub(20);
    e["requests"] = json!(reqs[keep..]);
    e["archived"] = json!(false);
    e["updated"] = json!(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0));
}

fn paste(c: &mut Client, id: u32, text: &str) -> Result<()> {
    let mut b = b"\x1b[200~".to_vec();
    b.extend_from_slice(text.as_bytes());
    b.extend_from_slice(b"\x1b[201~\r");
    c.send_input(id, &b)
}

pub fn run(cmd: DispCmd, mut c: Client) -> Result<()> {
    let mp = meta_path(&mut c)?;
    let mut meta = load(&mp);
    match cmd {
        DispCmd::List { project, all, json } => {
            let rs = rows(&mut c, &meta, project.as_deref())?;
            let overlap = |r: &Row| -> Vec<String> {
                rs.iter()
                    .filter(|o| o.path != r.path && !o.archived && !o.merged && o.files.intersection(&r.files).next().is_some())
                    .map(|o| o.branch.clone())
                    .collect()
            };
            let shown: Vec<&Row> = rs.iter().filter(|r| all || (!r.archived && !r.merged)).collect();
            if json {
                let v: Vec<Value> = shown
                    .iter()
                    .map(|r| {
                        json!({"path": r.path, "branch": r.branch, "title": r.title, "areas": r.areas, "agents": r.agents,
                            "merged": r.merged, "archived": r.archived, "last_request": r.last_request, "overlaps_with": overlap(r)})
                    })
                    .collect();
                println!("{}", serde_json::to_string_pretty(&v)?);
            } else if shown.is_empty() {
                println!("no open worktrees: create one with `axelstra dispatch new`");
            } else {
                for r in shown {
                    let state = if r.merged {
                        "merged"
                    } else if r.archived {
                        "archived"
                    } else if r.agents.is_empty() {
                        "no agent"
                    } else {
                        "agent running"
                    };
                    println!("{}  [{}]  {}", r.branch, state, r.title);
                    println!("    path: {}", r.path);
                    if !r.areas.is_empty() {
                        println!("    touches: {}", r.areas.join(" "));
                    }
                    if !r.last_request.is_empty() {
                        println!("    last request: {}", r.last_request);
                    }
                    let o = overlap(r);
                    if !o.is_empty() {
                        println!("    WARNING edits the same files as: {}", o.join(", "));
                    }
                }
            }
        }
        DispCmd::Send { worktree, request, agent, lead, no_lead } => {
            let rs = rows(&mut c, &meta, None)?;
            let r = match resolve(&rs, &worktree) {
                Ok(r) => r,
                Err(e) => {
                    let projects = c.call("project.list", json!({}))?;
                    return Err(main_checkout_named(&projects, &worktree).map(main_no_tasks).unwrap_or(e));
                }
            };
            if r.merged || r.archived {
                bail!(
                    "{} is {}: do not send new work to it. Use `axelstra dispatch new`",
                    r.branch,
                    if r.merged { "merged" } else { "archived" }
                );
            }
            let (path, branch) = (r.path.clone(), r.branch.clone());
            let live = c.call("session.list", json!({}))?;
            let existing = live
                .as_array()
                .map(|a| a.as_slice())
                .unwrap_or_default()
                .iter()
                .find(|s| is_main_agent(s) && norm(s["meta"]["worktree"].as_str().unwrap_or_default()) == norm(&path))
                .and_then(|s| s["id"].as_u64());
            if let Some(id) = existing {
                paste(&mut c, id as u32, &request)?;
                println!("sent to session {id} in {branch}");
            } else {
                println!("no agent was running in {branch}");
                let id = start(&mut c, &path, &agent, &request, choose_lead(lead, no_lead, &request))?;
                println!("started session {id}");
            }
            remember(&mut meta, &path, None, &request);
            save(&mp, &meta)?;
        }
        DispCmd::New { project, name, request, title, agent, lead, no_lead } => {
            let projects = c.call("project.list", json!({}))?;
            let pid = find_project_by_ref(&projects, &project)
                .and_then(|p| p["id"].as_str().map(String::from))
                .with_context(|| format!("no registered project matches {project:?}"))?;
            let w = c.call("worktree.create", json!({"project": pid, "name": name, "auto": true}))?;
            let path = w["path"].as_str().unwrap_or_default().to_string();
            let id = start(&mut c, &path, &agent, &request, choose_lead(lead, no_lead, &request))?;
            remember(&mut meta, &path, title.as_deref(), &request);
            save(&mp, &meta)?;
            println!("worktree {path}\nsession {id}");
        }
        DispCmd::Archive { worktree } => {
            let rs = rows(&mut c, &meta, None)?;
            let r = resolve(&rs, &worktree)?;
            let (path, name) = (r.path.clone(), r.branch.clone());
            entry(&mut meta, &path)["archived"] = json!(true);
            save(&mp, &meta)?;
            println!("archived {name}: it no longer receives requests");
        }
        DispCmd::Title { worktree, title } => {
            let rs = rows(&mut c, &meta, None)?;
            let path = resolve(&rs, &worktree)?.path.clone();
            entry(&mut meta, &path)["title"] = json!(title);
            save(&mp, &meta)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn lead_prompt_keeps_the_request_and_forbids_silent_waiting() {
        let p = super::lead_prompt("fix X");
        assert!(p.starts_with("fix X"));
        assert!(p.contains("axelstra done"));
        assert!(p.contains("Never wait silently for the human"));
        assert!(p.contains("axelstra ask $AXELSTRA_PARENT") && p.contains("axelstra escalate"));
    }

    #[test]
    fn lead_prompt_makes_the_lead_a_dispatcher_of_workers() {
        let p = super::lead_prompt("fix X");
        assert!(p.contains("dispatcher, not an implementer"));
        assert!(p.contains("axelstra agent . --prompt") && p.contains("--report-to $AXELSTRA_SESSION"));
        assert!(p.contains("do NOT edit code") && p.contains("axelstra lock verify"));
        assert!(p.contains("CARGO_BUILD_JOBS") && p.contains("cargo check"));
        assert!(!p.contains("you never execute it alone"));
        assert!(p.contains("commits, what was verified, what was not verified"));
    }

    use super::*;

    #[test]
    fn worker_prompt_does_the_work_and_integrates_it() {
        let p = worker_prompt("fix X");
        assert!(p.starts_with("fix X"));
        assert!(p.contains("WORKER") && p.contains("integrate it yourself"));
        assert!(p.contains("merge") && p.contains("axelstra lock verify -- node scripts/check.mjs"));
        assert!(p.contains("axelstra done") && p.contains("commits, what was verified, what was not verified"));
        assert!(p.contains("Never wait silently for the human"));
        assert!(p.contains("CARGO_BUILD_JOBS") && p.contains("cargo check"));
        assert!(!p.contains("dispatcher, not an implementer"));
    }

    #[test]
    fn send_to_the_main_checkout_is_refused_with_the_code() {
        let projects = json!([{"id": "p1", "name": "app", "path": "C:/x/app", "worktrees": [
            {"path": "C:/x/app", "branch": "main", "is_main": true},
            {"path": "C:/x/.axelstra-wt/app-fix", "branch": "ax/fix", "is_main": false}]}]);
        for want in ["C:/x/app", "C:/x/app/src", "main", "app", "p1"] {
            assert_eq!(main_checkout_named(&projects, want).as_deref(), Some("app"), "{want}");
        }
        for want in ["C:/x/.axelstra-wt/app-fix", "ax/fix", "fix", "C:/x/other"] {
            assert_eq!(main_checkout_named(&projects, want), None, "{want}");
        }
        let e = main_no_tasks("app".into());
        assert_eq!(axelstra_core::msg::find(&e).map(|c| c.code.as_str()), Some("session.main_no_tasks"));
        assert!(e.to_string().contains("axelstra dispatch new app <name>"));
    }

    #[test]
    fn wants_lead_needs_visible_independent_items() {
        assert!(!wants_lead("fix the typo in README"));
        assert!(!wants_lead("add a filter to the orders page and make sure it is fast"));
        assert!(!wants_lead("1. the only item"));
        assert!(!wants_lead("call f(1) then f(2) in the loop"));
        assert!(!wants_lead("bump to v2.1 - see notes"));
        assert!(wants_lead("do these:\n1. fix the login\n2. add a test"));
        assert!(wants_lead("- fix the login\n- add a test"));
        assert!(wants_lead("  * a thing\n  * another thing"));
        assert!(wants_lead("(1) fix the login (2) add a test"));
        assert!(wants_lead("First 1) fix login, then 2) add a test"));
        assert!(wants_lead("(1) fix login\n(2) add a test"));
    }

    #[test]
    fn flags_override_the_heuristic() {
        let many = "1. a\n2. b";
        assert!(choose_lead(false, false, many));
        assert!(!choose_lead(false, true, many));
        assert!(choose_lead(true, false, "one thing"));
        assert!(!choose_lead(false, false, "one thing"));
    }

    #[test]
    fn a_direct_worker_reaps_like_a_lead_but_is_not_one() {
        assert_eq!(lead_reap()["lead"], json!(true));
        assert!(worker_reap().get("lead").is_none());
        assert_eq!(worker_reap()["mode"], "exit");
        assert_eq!(lead_reap()["mode"], "exit");
    }

    #[test]
    fn areas_group_files_by_top_folders() {
        let f: BTreeSet<String> =
            ["app/src/a.ts", "app/src/b.ts", "crates/daemon/src/x.rs", "README.md", "docs/x.md"].iter().map(|s| s.to_string()).collect();
        assert_eq!(areas(&f), vec!["README.md", "app/src/", "crates/daemon/", "docs/"]);
    }

    #[test]
    fn remember_keeps_the_first_title_and_the_last_requests() {
        let mut m = json!({});
        remember(&mut m, "C:/wt/a", None, "add filter to orders");
        remember(&mut m, "C:/wt/a", None, "and sort it");
        assert_eq!(m["c:/wt/a"]["title"], "add filter to orders");
        assert_eq!(m["c:/wt/a"]["requests"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn changed_files_keeps_the_first_path_whole() {
        let dir = std::env::temp_dir().join(format!("axelstra-cf-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let d = dir.to_str().unwrap();
        let g = |a: &[&str]| {
            let ok = Command::new("git")
                .args(["-c", "user.name=t", "-c", "user.email=t@t"])
                .args(a)
                .current_dir(d)
                .output()
                .unwrap()
                .status
                .success();
            assert!(ok, "git {a:?}");
        };
        g(&["init", "-q"]);
        std::fs::write(dir.join("alpha.txt"), "1").unwrap();
        g(&["add", "."]);
        g(&["commit", "-qm", "first"]);
        std::fs::write(dir.join("alpha.txt"), "2").unwrap();
        std::fs::write(dir.join("new.txt"), "n").unwrap();
        let got = changed_files(d, "HEAD");
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(got.into_iter().collect::<Vec<_>>(), vec!["alpha.txt", "new.txt"]);
    }
}
