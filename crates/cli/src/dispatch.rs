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

const FOOTER: &str = "\n\nYou own this worktree and its topic. Later requests for the same topic are sent to you. When the work splits into independent parts, run `axelstra skills get axelstration` and delegate to workers; their sessions appear inside this worktree's box in the app.";

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
    let o = Command::new("git").args(args).current_dir(dir).output().ok()?;
    o.status.success().then(|| String::from_utf8_lossy(&o.stdout).trim().to_string())
}

/// true when GitHub says a pull request from this branch was merged (needs `gh`; any failure counts as no)
fn pr_merged(wt: &str, branch: &str) -> bool {
    git_like("gh", wt, &["pr", "list", "--head", branch, "--state", "merged", "--json", "number", "--limit", "1"])
        .map(|o| o.contains("number"))
        .unwrap_or(false)
}

fn git_like(program: &str, dir: &str, args: &[&str]) -> Option<String> {
    let o = Command::new(program).args(args).current_dir(dir).output().ok()?;
    o.status.success().then(|| String::from_utf8_lossy(&o.stdout).trim().to_string())
}

/// files this worktree changed: committed since the default branch plus uncommitted
fn changed_files(wt: &str, base: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    if let Some(t) = git(wt, &["diff", "--name-only", &format!("{base}...HEAD")]) {
        out.extend(t.lines().map(|l| l.trim().replace('\\', "/")).filter(|l| !l.is_empty()));
    }
    if let Some(t) = git(wt, &["status", "--porcelain"]) {
        out.extend(t.lines().filter_map(|l| l.get(3..)).map(|l| l.trim().replace('\\', "/")).filter(|l| !l.is_empty()));
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
        DispCmd::Send { worktree, request, agent } => {
            let rs = rows(&mut c, &meta, None)?;
            let r = resolve(&rs, &worktree)?;
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
                let id = spawn_agent(&mut c, Path::new(&path), &agent, Some(&format!("{request}{FOOTER}")), None, false)?;
                println!("no agent was running in {branch}: started session {id}");
            }
            remember(&mut meta, &path, None, &request);
            save(&mp, &meta)?;
        }
        DispCmd::New { project, name, request, title, agent } => {
            let projects = c.call("project.list", json!({}))?;
            let pid = find_project_by_ref(&projects, &project)
                .and_then(|p| p["id"].as_str().map(String::from))
                .with_context(|| format!("no registered project matches {project:?}"))?;
            let w = c.call("worktree.create", json!({"project": pid, "name": name}))?;
            let path = w["path"].as_str().unwrap_or_default().to_string();
            let id = spawn_agent(&mut c, Path::new(&path), &agent, Some(&format!("{request}{FOOTER}")), None, false)?;
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
    use super::*;

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
}
