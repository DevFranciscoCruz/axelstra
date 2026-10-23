use crate::agents;
use crate::card::{self, Card};
use crate::git::{self, Merge};
use crate::runner::{AgentSpawn, Handle, ProcessRunner, Runner};
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[derive(Clone)]
pub struct Options {
    pub repo: PathBuf,
    pub name: String,
    pub max_concurrency: usize,
    /// Parent dir for integration + task worktrees. Keep it short on Windows.
    pub wt_root: PathBuf,
    /// Program + args; `{prompt_file}` is substituted. Overrides per-card agent.
    pub agent_override: Option<Vec<String>>,
    pub poll_ms: u64,
    /// None = headless child processes.
    pub runner: Option<Arc<dyn Runner>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum St {
    Blocked,
    Ready,
    Running,
    Merged,
    Failed,
    NeedsReview,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "ev", rename_all = "snake_case")]
enum Ev {
    RunStart { base: String, integ_branch: String },
    Spawned { task: String, attempt: u32 },
    MergeBegin { task: String, integ_before: String },
    Merged { task: String },
    Failed { task: String, reason: String },
    NeedsReview { task: String, reason: String },
}

#[derive(Serialize, Deserialize)]
struct Line {
    t: u64,
    #[serde(flatten)]
    ev: Ev,
}

struct Log {
    f: File,
}

impl Log {
    fn append(&mut self, ev: Ev) -> Result<()> {
        let t = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
        writeln!(self.f, "{}", serde_json::to_string(&Line { t, ev })?)?;
        self.f.sync_data()?;
        Ok(())
    }
}

struct Task {
    card: Card,
    st: St,
    attempts: u32,
    live: Option<Live>,
}

struct Live {
    handle: Box<dyn Handle>,
    wt: PathBuf,
    branch: String,
    started: Instant,
}

#[derive(Deserialize)]
struct Result_ {
    status: String,
    #[serde(default)]
    summary: String,
}

pub struct Report {
    pub merged: Vec<String>,
    pub failed: Vec<(String, String)>,
    pub needs_review: Vec<(String, String)>,
    pub blocked: Vec<String>,
    pub integ_branch: String,
    pub integ_wt: PathBuf,
}

fn task_branch(run: &str, id: &str) -> String {
    // must not be nested under `orch/<run>`: a ref can't be both a file and a dir
    format!("orch-task/{run}/{id}")
}

/// Runs the cards. Cards that carry an `acceptance` command make this refuse:
/// the command goes through the shell, so the caller must opt in with `run_trusted`.
pub fn run(o: &Options) -> Result<Report> {
    run_with(o, false)
}

/// Like `run`, but the caller vouches for the cards' acceptance commands.
pub fn run_trusted(o: &Options) -> Result<Report> {
    run_with(o, true)
}

fn run_with(o: &Options, trust_acceptance: bool) -> Result<Report> {
    if !card::valid_slug(&o.name) {
        bail!("invalid run name {:?}: use letters, digits, '.', '_' or '-'", o.name);
    }
    let repo = git::canon(&o.repo)?;
    let common = git::common_dir(&repo)?;
    let run_dir = common.join("axelstra").join("orch").join(&o.name);
    std::fs::create_dir_all(run_dir.join("logs"))?;
    ensure_excluded(&common)?;

    let cards: Vec<Card> = card::load_dir(&repo.join(".kanban"))?.cards.into_iter().filter(|c| c.meta.status == "todo").collect();
    if cards.is_empty() {
        bail!("no cards with status: todo in {}", repo.join(".kanban").display());
    }
    // deps on cards outside `todo` (already done) are dropped, unknown ones are errors
    check_trust(&cards, trust_acceptance)?;
    let all = card::load_dir(&repo.join(".kanban"))?.cards;
    let todo_ids: Vec<String> = cards.iter().map(|c| c.meta.id.clone()).collect();
    let mut cards = cards;
    for c in &mut cards {
        c.meta.depends_on.retain(|d| todo_ids.contains(d) || !all.iter().any(|a| &a.meta.id == d && a.meta.status == "done"));
    }
    card::topo_order(&cards)?;

    let integ_branch = format!("orch/{}", o.name);
    let integ_wt = o.wt_root.join(format!("{}-integ", o.name));
    let events_path = run_dir.join("events.jsonl");
    // RunStart is only logged after the worktree exists, so an empty file means a failed start
    let resuming = std::fs::metadata(&events_path).map(|m| m.len() > 0).unwrap_or(false);
    let merged_before = if resuming { replay_merged(&events_path)? } else { Vec::new() };

    let mut log = Log { f: OpenOptions::new().create(true).append(true).open(&events_path)? };
    std::fs::create_dir_all(&o.wt_root)?;

    if !resuming {
        let base = git::head(&repo)?;
        git::worktree_add(&repo, &integ_wt, &integ_branch, &base)?;
        log.append(Ev::RunStart { base, integ_branch: integ_branch.clone() })?;
    } else {
        recover(&repo, &integ_wt, &events_path)?;
    }

    let mut tasks: HashMap<String, Task> = cards
        .into_iter()
        .map(|c| {
            let st = if merged_before.contains(&c.meta.id) { St::Merged } else { St::Blocked };
            (c.meta.id.clone(), Task { card: c, st, attempts: 0, live: None })
        })
        .collect();
    let mut failures: Vec<(String, String)> = Vec::new();
    let mut reviews: Vec<(String, String)> = Vec::new();

    loop {
        // 1. promote Blocked -> Ready once every dependency is Merged
        let merged_ids: Vec<String> = tasks.values().filter(|t| t.st == St::Merged).map(|t| t.card.meta.id.clone()).collect();
        for t in tasks.values_mut() {
            if t.st == St::Blocked && t.card.meta.depends_on.iter().all(|d| merged_ids.contains(d)) {
                t.st = St::Ready;
            }
        }

        // 2. poll running tasks
        let running: Vec<String> = tasks.values().filter(|t| t.st == St::Running).map(|t| t.card.meta.id.clone()).collect();
        for id in running {
            let t = tasks.get_mut(&id).unwrap();
            let live = t.live.as_mut().unwrap();
            let exited = live.handle.try_wait()?;
            // read after try_wait: an agent may write the result and exit between the two
            let sentinel = read_sentinel(&live.wt);
            let timeout = Duration::from_secs_f64(t.card.meta.timeout_min * 60.0);
            let outcome = if let Some(r) = sentinel {
                Some(if r.status == "done" { Ok(r.summary) } else { Err(format!("agent reported {}: {}", r.status, r.summary)) })
            } else if let Some(s) = exited {
                Some(Err(format!("{s} without result.json")))
            } else if live.started.elapsed() > timeout {
                Some(Err(format!("timeout after {} min", t.card.meta.timeout_min)))
            } else {
                None
            };
            let Some(outcome) = outcome else { continue };

            let live = t.live.take().unwrap();
            let mut live = live;
            live.handle.kill();

            match outcome {
                Ok(summary) => {
                    let why = finish_task(&repo, &integ_wt, &integ_branch, &o.name, t, &live.wt, &live.branch, &summary, &mut log)?;
                    if t.st == St::NeedsReview {
                        reviews.push((id.clone(), why.into()));
                    } else {
                        card::set_status(&t.card.path, "done")?;
                    }
                }
                Err(reason) => {
                    log.append(Ev::Failed { task: id.clone(), reason: reason.clone() })?;
                    let _ = git::worktree_remove(&repo, &live.wt);
                    let _ = git::branch_delete(&repo, &live.branch);
                    if t.attempts <= t.card.meta.max_retries {
                        t.st = St::Ready;
                    } else {
                        t.st = St::Failed;
                        failures.push((id.clone(), reason));
                    }
                }
            }
        }

        // 3. launch Ready tasks up to the concurrency cap, highest priority first
        let mut running_n = tasks.values().filter(|t| t.st == St::Running).count();
        let mut ready: Vec<String> = tasks.values().filter(|t| t.st == St::Ready).map(|t| t.card.meta.id.clone()).collect();
        ready.sort_by_key(|id| (-tasks[id].card.meta.priority, id.clone()));
        for id in ready {
            if running_n >= o.max_concurrency {
                break;
            }
            let t = tasks.get_mut(&id).unwrap();
            t.attempts += 1;
            log.append(Ev::Spawned { task: id.clone(), attempt: t.attempts })?;
            match spawn(&repo, &integ_branch, o, &run_dir, t) {
                Ok(live) => {
                    t.live = Some(live);
                    t.st = St::Running;
                    card::set_status(&t.card.path, "in-progress")?;
                    running_n += 1;
                }
                Err(e) => {
                    let reason = format!("spawn: {e:#}");
                    log.append(Ev::Failed { task: id.clone(), reason: reason.clone() })?;
                    t.st = St::Failed;
                    failures.push((id.clone(), reason));
                }
            }
        }

        // 4. stop when nothing can progress
        let merged_now = |id: &String| tasks.get(id).is_some_and(|t| t.st == St::Merged);
        let active = tasks.values().any(|t| {
            matches!(t.st, St::Running | St::Ready)
                // a merge in step 2 may have unblocked a task not promoted yet
                || (t.st == St::Blocked && t.card.meta.depends_on.iter().all(merged_now))
        });
        if !active {
            break;
        }
        std::thread::sleep(Duration::from_millis(o.poll_ms));
    }

    for t in tasks.values_mut() {
        if t.st == St::Failed || t.st == St::NeedsReview {
            let s = if t.st == St::Failed { "backlog" } else { "needs-review" };
            card::set_status(&t.card.path, s).ok();
        } else if t.st == St::Blocked {
            card::set_status(&t.card.path, "todo").ok();
        }
    }
    let by = |s: St| {
        let mut v: Vec<String> = tasks.values().filter(|t| t.st == s).map(|t| t.card.meta.id.clone()).collect();
        v.sort();
        v
    };
    Ok(Report { merged: by(St::Merged), failed: failures, needs_review: reviews, blocked: by(St::Blocked), integ_branch, integ_wt })
}

fn check_trust(cards: &[Card], trusted: bool) -> Result<()> {
    let cmds: Vec<String> = cards.iter().filter_map(|c| c.meta.acceptance.as_ref().map(|a| format!("{}: {a}", c.meta.id))).collect();
    if !trusted && !cmds.is_empty() {
        bail!(
            "acceptance commands run through the shell and were not trusted; confirm them first:
{}",
            cmds.join(
                "
"
            )
        );
    }
    Ok(())
}

fn ensure_excluded(common: &Path) -> Result<()> {
    let p = common.join("info").join("exclude");
    std::fs::create_dir_all(p.parent().unwrap())?;
    let cur = std::fs::read_to_string(&p).unwrap_or_default();
    if !cur.lines().any(|l| l.trim() == ".axelstra/") {
        let mut f = OpenOptions::new().create(true).append(true).open(&p)?;
        writeln!(f, "\n.axelstra/")?;
    }
    Ok(())
}

fn read_sentinel(wt: &Path) -> Option<Result_> {
    let s = std::fs::read_to_string(wt.join(".axelstra").join("result.json")).ok()?;
    serde_json::from_str(&s).ok()
}

fn spawn(repo: &Path, integ_branch: &str, o: &Options, run_dir: &Path, t: &Task) -> Result<Live> {
    let id = &t.card.meta.id;
    if !card::valid_slug(id) || !card::valid_slug(&o.name) {
        bail!("invalid card id or run name");
    }
    let wt = o.wt_root.join(format!("{}-{}", o.name, id));
    let branch = task_branch(&o.name, id);
    // a retry starts from a clean worktree
    if wt.exists() {
        let _ = git::worktree_remove(repo, &wt);
    }
    let _ = git::branch_delete(repo, &branch);
    git::worktree_add(repo, &wt, &branch, integ_branch)?;

    let dot = wt.join(".axelstra");
    std::fs::create_dir_all(&dot)?;
    let prompt = dot.join("prompt.md");
    std::fs::write(
        &prompt,
        format!(
            "{}\n\n---\nWhen finished, write `.axelstra/result.json` as {{\"status\":\"done\",\"summary\":\"...\"}} \
             (or status \"blocked\" with a reason). Do not commit; the orchestrator commits. \
             Work only inside this directory.\n",
            t.card.body.trim()
        ),
    )?;

    let runner: Arc<dyn Runner> = o.runner.clone().unwrap_or_else(|| Arc::new(ProcessRunner));
    let spec: Vec<String> = match &o.agent_override {
        Some(v) => v.clone(),
        None => {
            let a = agents::require(&t.card.meta.agent)?;
            if a.locate().is_none() {
                bail!("{}", a.missing_message());
            }
            agents::resolve_argv(a, agent_command(a.id, &t.card.meta.permission_mode, runner.interactive())?)
        }
    };
    let pf = prompt.to_string_lossy().to_string();
    let spec: Vec<String> = spec.into_iter().map(|a| a.replace("{prompt_file}", &pf)).collect();

    let handle = runner.spawn(&AgentSpawn {
        program: spec[0].clone(),
        args: spec[1..].to_vec(),
        cwd: wt.clone(),
        env: vec![("AXELSTRA_TASK".into(), id.clone()), ("AXELSTRA_PROMPT_FILE".into(), pf)],
        label: format!("{}/{}", o.name, id),
        log: Some(run_dir.join("logs").join(format!("{id}.log"))),
    })?;
    Ok(Live { handle, wt, branch, started: Instant::now() })
}

/// Command line for a card's agent, from the agents table. An agent with no way to take a prompt in its TUI
/// runs one-shot even under an interactive runner.
fn agent_command(agent: &str, permission_mode: &str, interactive: bool) -> Result<Vec<String>> {
    if !card::PERMISSION_MODES.contains(&permission_mode) {
        bail!("unknown permission_mode '{permission_mode}'");
    }
    let a = agents::require(agent)?;
    let ask = "Read .axelstra/prompt.md and do the task described there.";
    let mut v = if interactive && a.interactive_prompt.is_some() {
        let mut v = a.interactive_argv(Some(ask), false);
        // the safe unattended flags go right after the program, before the prompt
        v.splice(1..1, a.unattended.iter().map(|s| s.to_string()));
        v
    } else {
        a.headless_argv(ask, true, false)
    };
    if a.id == "claude" {
        v.splice(1..1, ["--permission-mode".to_string(), permission_mode.to_string()]);
    }
    Ok(v)
}

#[allow(clippy::too_many_arguments)]
fn finish_task(
    repo: &Path,
    integ_wt: &Path,
    integ_branch: &str,
    run: &str,
    t: &mut Task,
    wt: &Path,
    branch: &str,
    summary: &str,
    log: &mut Log,
) -> Result<&'static str> {
    let id = t.card.meta.id.clone();
    git::commit_all(wt, &format!("{id}: {}", first_line(summary, &t.card.meta.title)))?;
    let before = git::head(integ_wt)?;
    log.append(Ev::MergeBegin { task: id.clone(), integ_before: before.clone() })?;
    match git::merge(integ_wt, branch, &format!("merge {id}"))? {
        Merge::Conflict => {
            git::reset_hard(integ_wt, &before)?;
            log.append(Ev::NeedsReview { task: id.clone(), reason: "merge conflict".into() })?;
            t.st = St::NeedsReview; // worktree + branch stay for inspection
            return Ok("merge conflict");
        }
        Merge::Clean => {
            if let Some(cmd) = &t.card.meta.acceptance {
                if !run_acceptance(integ_wt, cmd)? {
                    git::reset_hard(integ_wt, &before)?;
                    log.append(Ev::NeedsReview { task: id.clone(), reason: "acceptance failed".into() })?;
                    t.st = St::NeedsReview;
                    return Ok("acceptance failed");
                }
            }
            log.append(Ev::Merged { task: id.clone() })?;
            t.st = St::Merged;
            let _ = git::worktree_remove(repo, wt);
            let _ = git::branch_delete(repo, &task_branch(run, &id));
            let _ = integ_branch;
        }
    }
    Ok("")
}

/// Merges a `needs-review` task again after the user fixed its branch (resolved the
/// conflict or the failing acceptance) in the task worktree. Cleans up on success.
pub fn retry_merge(repo: &Path, wt_root: &Path, name: &str, id: &str, trust_acceptance: bool) -> Result<()> {
    if !card::valid_slug(name) || !card::valid_slug(id) {
        bail!("invalid card id or run name");
    }
    let repo = git::canon(repo)?;
    let card = card::load_dir(&repo.join(".kanban"))?
        .cards
        .into_iter()
        .find(|c| c.meta.id == id)
        .ok_or_else(|| anyhow::anyhow!("card {id} not found"))?;
    if card.meta.status != "needs-review" {
        bail!("card {id} is not waiting for review");
    }
    check_trust(std::slice::from_ref(&card), trust_acceptance)?;
    let wt = wt_root.join(format!("{name}-{id}"));
    let integ_wt = wt_root.join(format!("{name}-integ"));
    if !wt.exists() || !integ_wt.exists() {
        bail!("worktrees of run {name} are gone");
    }
    if git::has_changes(&integ_wt)? {
        bail!("integration worktree has uncommitted changes");
    }
    let common = git::common_dir(&repo)?;
    let events = common.join("axelstra").join("orch").join(name).join("events.jsonl");
    let mut log = Log { f: OpenOptions::new().create(true).append(true).open(&events)? };
    let branch = task_branch(name, id);
    git::commit_all(&wt, &format!("{id}: {}", card.meta.title))?;
    let before = git::head(&integ_wt)?;
    log.append(Ev::MergeBegin { task: id.into(), integ_before: before.clone() })?;
    if let Merge::Conflict = git::merge(&integ_wt, &branch, &format!("merge {id}"))? {
        git::reset_hard(&integ_wt, &before)?;
        log.append(Ev::NeedsReview { task: id.into(), reason: "merge conflict".into() })?;
        bail!("still conflicts with the integration branch; resolve it in {}", wt.display());
    }
    if let Some(cmd) = &card.meta.acceptance {
        if !run_acceptance(&integ_wt, cmd)? {
            git::reset_hard(&integ_wt, &before)?;
            log.append(Ev::NeedsReview { task: id.into(), reason: "acceptance failed".into() })?;
            bail!("acceptance failed after merging");
        }
    }
    log.append(Ev::Merged { task: id.into() })?;
    card::set_status(&card.path, "done")?;
    let _ = git::worktree_remove(&repo, &wt);
    let _ = git::branch_delete(&repo, &branch);
    Ok(())
}

fn first_line(summary: &str, fallback: &str) -> String {
    let l = summary.lines().next().unwrap_or("").trim();
    if l.is_empty() {
        fallback.to_string()
    } else {
        l.to_string()
    }
}

fn run_acceptance(wt: &Path, cmd: &str) -> Result<bool> {
    let st = if cfg!(windows) {
        git::quiet(&mut Command::new("cmd")).args(["/d", "/c", cmd]).current_dir(wt).status()?
    } else {
        Command::new("sh").args(["-c", cmd]).current_dir(wt).status()?
    };
    Ok(st.success())
}

fn replay_merged(events: &Path) -> Result<Vec<String>> {
    let mut out = Vec::new();
    for l in std::fs::read_to_string(events)?.lines() {
        if let Ok(Line { ev: Ev::Merged { task }, .. }) = serde_json::from_str::<Line>(l) {
            out.push(task);
        }
    }
    Ok(out)
}

/// A crash between `merge_begin` and `merged` can leave a half-done merge:
/// roll the integration worktree back to where it was before that merge.
fn recover(repo: &Path, integ_wt: &Path, events: &Path) -> Result<()> {
    let mut pending: Option<String> = None;
    for l in std::fs::read_to_string(events)?.lines() {
        match serde_json::from_str::<Line>(l).map(|l| l.ev) {
            Ok(Ev::MergeBegin { integ_before, .. }) => pending = Some(integ_before),
            Ok(Ev::Merged { .. }) | Ok(Ev::NeedsReview { .. }) => pending = None,
            _ => {}
        }
    }
    if !integ_wt.exists() {
        bail!("run exists but integration worktree {} is gone", integ_wt.display());
    }
    if let Some(sha) = pending {
        if git::merge_in_progress(integ_wt) {
            let _ = git::run(integ_wt, &["merge", "--abort"]);
        }
        git::reset_hard(integ_wt, &sha)?;
    }
    let _ = git::run(repo, &["worktree", "prune"]);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_command_follows_the_agents_table() {
        let p = "Read .axelstra/prompt.md and do the task described there.";
        let c = agent_command("claude", "plan", false).unwrap();
        assert_eq!(c, ["claude", "--permission-mode", "plan", "-p", p]);
        assert_eq!(agent_command("claude", "plan", true).unwrap(), ["claude", "--permission-mode", "plan", p]);
        let x = agent_command("chatgpt", "auto", false).unwrap();
        assert_eq!(x, ["codex", "exec", "--sandbox", "workspace-write", p]);
        assert_eq!(agent_command("codex", "auto", true).unwrap(), ["codex", "--sandbox", "workspace-write", p]);
        assert_eq!(agent_command("gemini", "auto", false).unwrap(), ["gemini", "--approval-mode", "auto_edit", "-p", p]);
        assert_eq!(agent_command("gemini", "auto", true).unwrap(), ["gemini", "--approval-mode", "auto_edit", "-i", p]);
        assert!(agent_command("rm", "auto", false).unwrap_err().to_string().contains("desconhecido"));
        assert!(agent_command("claude", "--evil", false).is_err());
    }
}
