//! Removes the worktrees Axelstra's agents created (branches `ax/...`, and an orchestrator run's `orch/...` and
//! `orch-task/...`) once their work is integrated: the branch is in
//! the default branch already, it is inside another live `ax/*` branch (stacked integration), or GitHub says its pull
//! request was merged (squash merges leave no ancestor). A worktree with a running session that has not reported,
//! uncommitted changes or unmerged work is never touched, and neither is a branch whose commits no other ref holds.
//! Safe by default: a worktree with no `auto` mark (hand-made, old, migrated) goes only through real integration of its
//! own commits; an empty one is removed automatically only when it is `auto`. Nothing goes while a session was in it
//! within `MANUAL_QUIET_SECS`.

use crate::integrate;
use crate::projects::{self, Project};
use anyhow::Result;
use axelstra_core::exec::Captured;
use axelstra_core::git;
use serde_json::{json, Value};
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

/// Longest a network call (`git fetch`, `gh`) of the sweep may take; the scheduler loop must not wait on a hung one.
const NET_TIMEOUT: Duration = Duration::from_secs(30);

/// Output cap of the sweep's network calls.
const NET_MAX_OUT: usize = 1 << 20;

/// Runs `cmd` and kills its whole tree after `limit`; `None` when it cannot start or ran out of time.
pub(crate) fn output_within(cmd: &mut Command, limit: Duration) -> Option<Captured> {
    axelstra_core::exec::run_capped(cmd, limit, NET_MAX_OUT).ok().filter(|o| !o.timed_out)
}

/// Why this branch counts as merged, `None` when it does not (or cannot be told).
pub fn merged_reason(repo: &Path, branch: &str, default: &str) -> Option<String> {
    merged_reason_in(repo, branch, default, &[])
}

/// `merged_reason`, also counting the branch as integrated when its tip is inside one of `others` (the other live
/// `ax/*` branches): its commits stay reachable from there, so removing it loses nothing.
pub fn merged_reason_in(repo: &Path, branch: &str, default: &str, others: &[String]) -> Option<String> {
    merged_reason_with(repo, branch, default, others, true)
}

/// `merged_reason_in` for readers that must never start a process for GitHub (the worktree overview, polled by the app):
/// git only, plus the answer `gh` gave the last time a sweep or cleanup asked, if it was for this same tip.
pub fn merged_reason_cached(repo: &Path, branch: &str, default: &str, others: &[String]) -> Option<String> {
    merged_reason_with(repo, branch, default, others, false)
}

/// Branches whose merged PR `gh` reported, by repo and branch: the tip it was for and the PR number. Filled by the
/// callers that may run `gh` (the sweep, cleanup), read by the overview; a branch that moved no longer matches.
type GhMerged = std::collections::HashMap<(String, String), (String, u64)>;
static GH_MERGED: Mutex<Option<GhMerged>> = Mutex::new(None);

fn gh_key(repo: &Path, branch: &str) -> (String, String) {
    (repo.to_string_lossy().into_owned(), branch.to_string())
}

fn gh_merged_note(repo: &Path, branch: &str, tip: &str, number: Option<u64>) {
    let mut m = GH_MERGED.lock().unwrap_or_else(|e| e.into_inner());
    let map = m.get_or_insert_with(Default::default);
    match number {
        Some(n) => {
            map.insert(gh_key(repo, branch), (tip.to_string(), n));
        }
        None => {
            map.remove(&gh_key(repo, branch));
        }
    }
}

fn gh_merged_known(repo: &Path, branch: &str, tip: &str) -> Option<u64> {
    let m = GH_MERGED.lock().unwrap_or_else(|e| e.into_inner());
    m.as_ref()?.get(&gh_key(repo, branch)).filter(|(t, _)| t == tip).map(|(_, n)| *n)
}

/// What git alone says about a branch (no GitHub): it reads only commits and refs, so it is kept while the repo's
/// ref fingerprint holds (`gitstamp::refs`); the sweep and the overview then ask git again only after a ref moved.
#[derive(Clone)]
enum GitVerdict {
    /// no commit of its own (or git could not tell its tip): never "merged"
    NotWork,
    Merged(String),
    /// work that git does not find anywhere else, at this tip
    Open(String),
}

static VERDICTS: axelstra_core::gitstamp::Stamped<GitVerdict> = axelstra_core::gitstamp::Stamped::new();

fn git_verdict(repo: &Path, branch: &str, default: &str, others: &[String]) -> GitVerdict {
    let key = format!("{}|{branch}|{default}|{}", repo.to_string_lossy(), others.join("|"));
    VERDICTS.get(&key, axelstra_core::gitstamp::refs(repo), None, || git_verdict_uncached(repo, branch, default, others))
}

fn git_verdict_uncached(repo: &Path, branch: &str, default: &str, others: &[String]) -> GitVerdict {
    // a branch nobody committed to is "inside" the default branch too, and must not be mistaken for merged work
    if !has_own_commits(repo, branch) {
        return GitVerdict::NotWork;
    }
    let Some(tip) = git::run(repo, &["rev-parse", branch]).ok().filter(|o| o.ok).map(|o| o.stdout.trim().to_string()) else {
        return GitVerdict::NotWork;
    };
    let upstream = format!("origin/{default}");
    if git::run(repo, &["merge-base", "--is-ancestor", branch, &upstream]).map(|o| o.ok).unwrap_or(false) {
        return GitVerdict::Merged(format!("already in {upstream}"));
    }
    // merged into the local default branch but not pushed yet: the commits stay reachable from it, so removing the branch loses nothing
    if git::run(repo, &["merge-base", "--is-ancestor", branch, default]).map(|o| o.ok).unwrap_or(false) {
        return GitVerdict::Merged(format!("already in local {default}"));
    }
    for other in others.iter().filter(|o| o.as_str() != branch) {
        if git::run(repo, &["merge-base", "--is-ancestor", branch, other]).map(|o| o.ok).unwrap_or(false) {
            return GitVerdict::Merged(format!("already in {other}"));
        }
    }
    // squash and rebase merges leave no ancestor: every commit has an equivalent in the default branch
    if crate::cleanup::squash_merged(repo, branch, &upstream) || crate::cleanup::squash_merged(repo, branch, default) {
        return GitVerdict::Merged(format!("its changes are already in {default}"));
    }
    GitVerdict::Open(tip)
}

fn merged_reason_with(repo: &Path, branch: &str, default: &str, others: &[String], ask_gh: bool) -> Option<String> {
    let tip = match git_verdict(repo, branch, default, others) {
        GitVerdict::NotWork => return None,
        GitVerdict::Merged(why) => return Some(why),
        GitVerdict::Open(tip) => tip,
    };
    if !ask_gh {
        return gh_merged_known(repo, branch, &tip).map(|n| format!("PR #{n} merged"));
    }
    let o = output_within(
        Command::new("gh").current_dir(repo).env("GH_PROMPT_DISABLED", "1").args([
            "pr",
            "list",
            "--state",
            "merged",
            "--head",
            branch,
            "--limit",
            "1",
            "--json",
            "number,headRefOid",
        ]),
        NET_TIMEOUT,
    )
    .filter(|o| o.success())?;
    let v: Value = serde_json::from_slice(&o.stdout).ok()?;
    let pr = v.as_array()?.first()?.clone();
    // the merged PR must be exactly what is here: commits added after the merge would be lost with the branch
    if pr["headRefOid"].as_str() != Some(tip.as_str()) {
        gh_merged_note(repo, branch, &tip, None);
        return None;
    }
    let n = pr["number"].as_u64();
    gh_merged_note(repo, branch, &tip, n);
    n.map(|n| format!("PR #{n} merged"))
}

/// True for the branches Axelstra's own flows create and the janitor may clean up: agent worktrees (`ax/*`), an
/// orchestrator run's integration branch (`orch/*`) and its task branches (`orch-task/*`).
pub fn managed_branch(b: &str) -> bool {
    b.starts_with("ax/") || integrable_branch(b) || b.starts_with("orch-task/")
}

/// The managed branches whose work is integrated into the default branch (`ax/*` and `orch/*`); a task branch of a run
/// goes into its integration branch instead.
pub fn integrable_branch(b: &str) -> bool {
    b.starts_with("ax/") || b.starts_with("orch/")
}

/// True when the branch's reflog records a commit made on it (a fresh branch only has its creation). An integration
/// branch of the orchestrator only ever receives merges (`merge --no-ff`), so for `orch/*` a merge counts too.
pub fn has_own_commits(repo: &Path, branch: &str) -> bool {
    let merges = branch.starts_with("orch/");
    git::run(repo, &["reflog", "show", "--format=%gs", branch])
        .map(|o| o.ok && o.stdout.lines().any(|l| l.starts_with("commit") || (merges && l.starts_with("merge"))))
        .unwrap_or(false)
}

/// The branch a `why` from `merged_reason_in` says the work went into ("main", "ax/other"), for the notice.
fn integrated_into(why: &str, default: &str) -> String {
    match why.strip_prefix("already in ") {
        Some(rest) => rest.strip_prefix("local ").or_else(|| rest.strip_prefix("origin/")).unwrap_or(rest).to_string(),
        None => default.to_string(),
    }
}

static EVENTS: Mutex<Vec<Value>> = Mutex::new(Vec::new());

/// Takes (and clears) the worktrees the sweep removed since the last call, for the app's toast and notification center.
pub fn take_events() -> Vec<Value> {
    std::mem::take(&mut *EVENTS.lock().unwrap_or_else(|e| e.into_inner()))
}

/// Whether the app has events to take (it is told over the watch stream, see `Registry::announce_events`).
pub fn has_events() -> bool {
    !EVENTS.lock().unwrap_or_else(|e| e.into_inner()).is_empty()
}

/// Drops the queued events of a project that was removed (nothing is left to show them for).
pub fn drop_events_of(project: &str) {
    EVENTS.lock().unwrap_or_else(|e| e.into_inner()).retain(|e| e["project"].as_str() != Some(project));
}

pub(crate) fn push_event(e: Value) {
    let mut q = EVENTS.lock().unwrap_or_else(|e| e.into_inner());
    q.push(e);
    let n = q.len();
    if n > 100 {
        q.drain(..n - 100);
    }
}

/// How old an empty worktree (no agent, no changes, no commits of its own) must be before the sweep removes it, so one the
/// user just made by hand is left alone. A worktree whose session reported `done` goes at the next sweep.
const EMPTY_MIN_AGE_SECS: u64 = 10 * 60;

/// Retry delay for a removal that stayed pending (a process outside our sessions holds the folder, so no exit event
/// reaches us): doubles from one minute up to 30 minutes. Sessions that exit kick the janitor by event (`watch_exit`),
/// so this is only the fallback for holders we cannot observe; it is not a fixed poll.
pub fn retry_delay(attempt: u32) -> Duration {
    Duration::from_secs((60u64 << attempt.saturating_sub(1).min(5)).min(30 * 60))
}

pub(crate) fn attention_event(p: &Project, path: &str, branch: &str, why: &axelstra_core::msg::Coded) -> Value {
    json!({"kind": "attention", "project": p.id, "path": path, "branch": branch, "code": why.code, "params": why.params_json(),
           "text": why.text, "at": now_ms()})
}

/// What the sweep needs to know about, and do to, the live sessions.
pub struct Env<'a> {
    /// a session works in this worktree (or a flow or a restore still needs it)
    pub busy: &'a dyn Fn(&str) -> bool,
    /// closes every session of this worktree, killing their process trees, and waits until they are gone; returns their labels
    pub close: &'a dyn Fn(&str) -> Vec<String>,
    /// a session of any kind is open in this worktree, or one was in the last `MANUAL_QUIET_SECS`
    pub touched: &'a dyn Fn(&str) -> bool,
}

/// How long a worktree the user made by hand must have had no session before the sweep may remove it.
pub const MANUAL_QUIET_SECS: u64 = 10 * 60;

/// Whether the last sweep left a worktree with work of its own that no branch holds yet: only then can something on
/// GitHub (a PR merged there) make it removable with no local event, so only then does the janitor keep its one
/// deadline (`JanitorTiming::slow`). With nothing waiting the janitor sleeps until an event.
static AWAITING_REMOTE: AtomicBool = AtomicBool::new(false);

pub fn awaiting_remote() -> bool {
    AWAITING_REMOTE.load(Ordering::SeqCst)
}

fn sweep_project(p: &Project, env: &Env, dry: bool, out: &mut Vec<Value>, waiting: &mut bool) -> Result<()> {
    let busy = env.busy;
    let repo = Path::new(&p.path);
    let default = git::default_branch(repo);
    let mut fetched = false;
    // branches already removed by this pass cannot keep another one's commits alive
    let mut gone: Vec<String> = vec![];
    for w in git::worktree_list(repo)? {
        let Some(branch) = w.branch.clone().filter(|b| managed_branch(b)) else { continue };
        if w.is_main || busy(&w.path) {
            // an agent works here again: whatever kept the last report from being integrated is moot
            if !w.is_main {
                integrate::clear_attention(&w.path);
                pending_end(&w.path);
            }
            continue;
        }
        // safe by default: only a worktree marked `auto` (made by `dispatch new`) is automatic; no mark means manual
        let auto = p.auto.contains(&axelstra_core::paths::norm(&w.path)) && !p.manual.contains(&axelstra_core::paths::norm(&w.path));
        // nothing is removed while a session is in it or was a moment ago; an auto worktree whose agent just said `done`
        // is the one exception (the report is the signal that the work is finished)
        if (env.touched)(&w.path) && !(auto && integrate::done_mark(&w.path, false).is_some()) {
            continue;
        }
        let dirty = git::has_changes(Path::new(&w.path)).unwrap_or(true);
        // a lead (or `--report-to` worker) said `done` here: integrate its work, or flag why that is not possible
        let reported = if dry { integrate::done_mark(&w.path, false) } else { integrate::done_mark(&w.path, true) };
        if let (Some(ok), false) = (reported, dry) {
            if integrate::auto_on(p) {
                let result = if ok {
                    integrate::integrate_with(p, &w, &|| (env.close)(&w.path))
                } else {
                    Err(axelstra_core::coded!(
                        "integrate.not_ok",
                        "the agent reported failed or blocked, so {branch} was not integrated",
                        branch = branch
                    ))
                };
                match result {
                    Ok(integrate::Outcome::Integrated { into, how, pushed, removed, push_error, closed, .. }) => {
                        integrate::clear_attention(&w.path);
                        if removed {
                            gone.push(branch.clone());
                        }
                        let why = format!("integrated into {into} ({how})");
                        let mut ev = removed_event(p, &w.path, &branch, &into, &why, &closed);
                        ev["kind"] = json!("integrated");
                        ev["how"] = json!(how);
                        ev["pushed"] = json!(pushed);
                        ev["removed"] = json!(removed);
                        push_event(ev);
                        if let Some(c) = push_error {
                            push_event(attention_event(p, &w.path, &branch, &c));
                        }
                        out.push(json!({"project": p.id, "path": w.path, "branch": branch, "into": into, "why": why, "removed": removed, "closed": closed, "dry_run": false}));
                        continue;
                    }
                    Ok(integrate::Outcome::Nothing) => {}
                    Err(e) => {
                        let why = integrate::as_coded(&e, &branch);
                        integrate::set_attention(&w.path, why.clone());
                        push_event(attention_event(p, &w.path, &branch, &why));
                        out.push(json!({"project": p.id, "path": w.path, "branch": branch, "why": why.text, "code": why.code, "removed": false, "dry_run": false}));
                        continue;
                    }
                }
            }
        }
        if dirty {
            *waiting |= has_own_commits(repo, &branch);
            continue;
        }
        if !fetched {
            // so a merge done on GitHub since the last fetch is seen; a failure (offline) just means less is found
            let _ = output_within(
                Command::new("git").current_dir(repo).env("GIT_TERMINAL_PROMPT", "0").args(["fetch", "--quiet", "origin"]),
                NET_TIMEOUT,
            );
            fetched = true;
        }
        // a live `ax/*` branch is one that still exists in a worktree (a dry run removes none, so `gone` stays empty)
        let others: Vec<String> = git::worktree_list(repo)?
            .into_iter()
            .filter_map(|o| o.branch)
            .filter(|b| integrable_branch(b) && *b != branch && !gone.contains(b))
            .collect();
        let Some(why) = merged_reason_in(repo, &branch, &default, &others) else {
            *waiting |= has_own_commits(repo, &branch);
            // nothing of its own and nothing to lose: an empty worktree nobody works in goes away after a few minutes
            // only an `auto` one: a manual or unmarked one may just be waiting for its first commit, and leaves only by
            // the user's button
            let old = auto && (reported.is_some() || integrate::branch_age_secs(repo, &branch).is_some_and(|a| a >= EMPTY_MIN_AGE_SECS));
            if old && integrate::ahead_of(repo, &branch, &default) == 0 && !has_own_commits(repo, &branch) {
                let (removed, closed) = if dry { (false, vec![]) } else { remove_closing(env, p, &w.path, &branch) };
                if removed {
                    gone.push(branch.clone());
                    push_event(
                        json!({"kind": "empty", "project": p.id, "path": w.path, "branch": branch, "closed": closed, "at": now_ms()}),
                    );
                }
                out.push(json!({"project": p.id, "path": w.path, "branch": branch, "why": "empty", "removed": removed, "pending": !dry && !removed, "dry_run": dry}));
            }
            continue;
        };
        let (removed, closed) = if dry { (false, vec![]) } else { remove_closing(env, p, &w.path, &branch) };
        let into = integrated_into(&why, &default);
        if removed {
            gone.push(branch.clone());
            push_event(removed_event(p, &w.path, &branch, &into, &why, &closed));
        }
        out.push(json!({"project": p.id, "path": w.path, "branch": branch, "into": into, "why": why, "removed": removed, "closed": closed, "pending": !dry && !removed, "dry_run": dry}));
    }
    Ok(())
}

/// Closes the sessions still in the worktree (they hold its folder open on Windows), then removes it all-or-nothing:
/// when the folder is still held nothing was removed, git and the branch are untouched and the next sweep (kicked when
/// the holder exits, and again shortly after a failure) tries again. Returns whether it went, and the sessions closed.
/// A removal that failed because the folder is held is reported once (`pending_event`); one that went ends the episode.
fn remove_closing(env: &Env, p: &Project, path: &str, branch: &str) -> (bool, Vec<String>) {
    let closed = (env.close)(path);
    // a session closed on the way may have left something behind: then the work is not finished after all
    if !closed.is_empty() && git::has_changes(Path::new(path)).unwrap_or(true) {
        return (false, closed);
    }
    match projects::worktree_remove_strict(p, path) {
        Ok(()) => {
            pending_end(path);
            (true, closed)
        }
        Err(e) => {
            if pending_begin(path) {
                push_event(pending_event(p, path, branch, &e, holder_of(Path::new(path))));
            }
            (false, closed)
        }
    }
}

/// Worktrees whose removal stayed pending and that were already reported: the retries of one episode say nothing new.
static PENDING: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// True when `path` starts a new pending episode (it was not reported yet).
fn pending_begin(path: &str) -> bool {
    let k = axelstra_core::paths::norm(path);
    let mut s = PENDING.lock().unwrap_or_else(|e| e.into_inner());
    if s.contains(&k) {
        return false;
    }
    s.push(k);
    true
}

/// The episode of `path` is over (it was removed, or a session works in it again).
fn pending_end(path: &str) {
    let k = axelstra_core::paths::norm(path);
    PENDING.lock().unwrap_or_else(|e| e.into_inner()).retain(|p| *p != k);
}

/// The janitor event of an integrated worktree whose folder could not be removed: `janitor.pending` names the process that
/// holds it when one is found, else `janitor.pending_busy` carries the reason of the failed removal (`reason` its code).
pub(crate) fn pending_event(p: &Project, path: &str, branch: &str, err: &anyhow::Error, holder: Option<(String, u32)>) -> Value {
    let short = branch.trim_start_matches("ax/").to_string();
    let reason = axelstra_core::msg::find(err).map(|c| c.code.clone());
    let c = match &holder {
        Some((name, pid)) => axelstra_core::msg::Coded::new(
            "janitor.pending",
            format!("worktree {short} is integrated but {name} (pid {pid}) holds its folder {path}; it goes when that process exits"),
            &[("branch", short.clone()), ("path", path.to_string()), ("holder", name.clone()), ("pid", pid.to_string())],
        ),
        None => {
            let detail = axelstra_core::msg::find(err).map(|c| c.text.clone()).unwrap_or_else(|| format!("{err:#}"));
            axelstra_core::msg::Coded::new(
                "janitor.pending_busy",
                format!("worktree {short} is integrated but its folder {path} could not be removed yet: {detail}"),
                &[("branch", short.clone()), ("path", path.to_string()), ("detail", detail.clone())],
            )
        }
    };
    let holder = holder.map(|(name, pid)| json!({"name": name, "pid": pid}));
    json!({"kind": "pending", "project": p.id, "path": path, "branch": branch, "holder": holder, "reason": reason,
           "code": c.code, "params": c.params_json(), "text": c.text, "at": now_ms()})
}

/// A process that keeps `dir` from being removed, when one can be found cheaply: on Linux one whose working directory or
/// executable is inside it (`/proc`), on Windows one whose executable is inside it (a test binary or a dev server built
/// there). A handle opened by another process (an editor, an indexer) is not found: then the reason is all there is.
pub fn holder_of(dir: &Path) -> Option<(String, u32)> {
    let d = git::canon(dir).unwrap_or_else(|_| dir.to_path_buf()).to_string_lossy().to_string();
    let me = std::process::id();
    holder_in(&d, me)
}

#[cfg(target_os = "linux")]
fn holder_in(dir: &str, me: u32) -> Option<(String, u32)> {
    for e in std::fs::read_dir("/proc").ok()?.flatten() {
        let Some(pid) = e.file_name().to_str().and_then(|n| n.parse::<u32>().ok()) else { continue };
        if pid == me {
            continue;
        }
        let inside = ["cwd", "exe"]
            .iter()
            .any(|l| std::fs::read_link(e.path().join(l)).is_ok_and(|t| axelstra_core::paths::is_within(&t.to_string_lossy(), dir)));
        if inside {
            let name = std::fs::read_to_string(e.path().join("comm")).map(|n| n.trim().to_string()).unwrap_or_default();
            return Some((name, pid));
        }
    }
    None
}

#[cfg(windows)]
fn holder_in(dir: &str, me: u32) -> Option<(String, u32)> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::ProcessStatus::EnumProcesses;
    use windows_sys::Win32::System::Threading::{OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION};
    let mut pids = vec![0u32; 4096];
    let mut got = 0u32;
    if unsafe { EnumProcesses(pids.as_mut_ptr(), (pids.len() * 4) as u32, &mut got) } == 0 {
        return None;
    }
    pids.truncate(got as usize / 4);
    for pid in pids.into_iter().filter(|&p| p != 0 && p != me) {
        let image = unsafe {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if h.is_null() {
                continue;
            }
            let mut buf = [0u16; 1024];
            let mut len = buf.len() as u32;
            let ok = QueryFullProcessImageNameW(h, 0, buf.as_mut_ptr(), &mut len);
            CloseHandle(h);
            if ok == 0 {
                continue;
            }
            String::from_utf16_lossy(&buf[..len as usize])
        };
        if axelstra_core::paths::is_within(&image, dir) {
            return Some((image.rsplit(['\\', '/']).next().unwrap_or("").to_string(), pid));
        }
    }
    None
}

#[cfg(not(any(windows, target_os = "linux")))]
fn holder_in(_dir: &str, _me: u32) -> Option<(String, u32)> {
    None
}

/// The toast / notification of a removal; with closed sessions it names them (`janitor.removed_closed`).
pub(crate) fn removed_event(p: &Project, path: &str, branch: &str, into: &str, why: &str, closed: &[String]) -> Value {
    let mut e = json!({"project": p.id, "path": path, "branch": branch, "into": into, "why": why, "closed": closed, "at": now_ms()});
    if !closed.is_empty() {
        let (branch, sessions) = (branch.trim_start_matches("ax/"), closed.join(", "));
        let c = axelstra_core::msg::Coded::new(
            "janitor.removed_closed",
            format!("worktree {branch} removed (merged into {into}); session {sessions} closed"),
            &[("branch", branch.to_string()), ("into", into.to_string()), ("sessions", sessions)],
        );
        e["code"] = json!(c.code);
        e["params"] = c.params_json();
        e["text"] = json!(c.text);
    }
    e
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// Sweeps every project; `busy(path)` says whether a live session works in that worktree.
pub fn sweep(store: &Path, busy: &dyn Fn(&str) -> bool, dry: bool) -> Vec<Value> {
    sweep_with(store, &Env { busy, close: &|_| vec![], touched: &|_| false }, dry)
}

/// `sweep` with a way to close sessions and to tell hand-made worktrees in use.
pub fn sweep_with(store: &Path, env: &Env, dry: bool) -> Vec<Value> {
    let busy = env.busy;
    let mut out = vec![];
    let all = projects::load(store);
    let mut waiting = false;
    // a pending worktree removed some other way (the user's button) ends its episode too
    PENDING.lock().unwrap_or_else(|e| e.into_inner()).retain(|k| Path::new(k).exists());
    for p in &all {
        let _ = sweep_project(p, env, dry, &mut out, &mut waiting);
        crate::cleanup::stale_branches(p, dry, &mut out);
        crate::cleanup::prune_entries(p, dry, &mut out);
    }
    if !dry {
        AWAITING_REMOTE.store(waiting, Ordering::SeqCst);
    }
    crate::cleanup::orphan_dirs(&all, busy, dry, &mut out);
    if let Some(dir) = store.parent() {
        crate::cleanup::rotate(dir, dry, &mut out);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn the_cached_reason_never_asks_gh_and_follows_the_tip() {
        let dir = std::env::temp_dir().join(format!("axl-jan-gh-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let g = |args: &[&str]| assert!(git::run(&dir, args).unwrap().ok, "git {args:?}");
        let commit = ["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "--allow-empty", "-m", "x"];
        g(&["init", "-q", "-b", "main"]);
        g(&commit);
        g(&["update-ref", "refs/remotes/origin/main", "HEAD"]);
        g(&["checkout", "-q", "-b", "ax/sq"]);
        g(&commit);
        g(&commit);
        // no answer from gh yet: the overview's read says "not merged" without starting anything
        assert!(merged_reason_cached(&dir, "ax/sq", "main", &[]).is_none());
        let tip = git::run(&dir, &["rev-parse", "ax/sq"]).unwrap().stdout.trim().to_string();
        gh_merged_note(&dir, "ax/sq", &tip, Some(7));
        assert_eq!(merged_reason_cached(&dir, "ax/sq", "main", &[]).as_deref(), Some("PR #7 merged"));
        // the branch moved after the PR: the kept answer no longer applies
        g(&commit);
        assert!(merged_reason_cached(&dir, "ax/sq", "main", &[]).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_hung_command_is_killed_at_the_limit() {
        let (prog, args): (&str, &[&str]) = if cfg!(windows) { ("ping", &["-n", "30", "127.0.0.1"]) } else { ("sleep", &["30"]) };
        let t = Instant::now();
        assert!(output_within(Command::new(prog).args(args), Duration::from_millis(300)).is_none());
        assert!(t.elapsed() < Duration::from_secs(10));
        let (prog, args): (&str, &[&str]) = if cfg!(windows) { ("cmd", &["/C", "echo", "hi"]) } else { ("echo", &["hi"]) };
        let o = output_within(Command::new(prog).args(args), Duration::from_secs(10)).unwrap();
        assert!(o.success() && o.stdout_lossy().contains("hi"));
    }

    #[test]
    fn a_branch_inside_the_default_branch_is_merged_and_a_new_one_is_not() {
        let dir = std::env::temp_dir().join(format!("axl-jan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let g = |args: &[&str]| assert!(git::run(&dir, args).unwrap().ok, "git {args:?}");
        let commit = ["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "--allow-empty", "-m", "x"];
        g(&["init", "-q", "-b", "main"]);
        g(&commit);
        // a local "origin/main" stands in for the remote
        g(&["update-ref", "refs/remotes/origin/main", "HEAD"]);
        // created and never committed to: not merged work
        g(&["branch", "ax/fresh"]);
        assert!(merged_reason(&dir, "ax/fresh", "main").is_none());
        g(&["checkout", "-q", "-b", "ax/done"]);
        g(&commit);
        g(&["checkout", "-q", "main"]);
        g(&["merge", "-q", "--ff-only", "ax/done"]);
        g(&["update-ref", "refs/remotes/origin/main", "HEAD"]);
        g(&["checkout", "-q", "-b", "ax/new"]);
        g(&commit);
        assert_eq!(merged_reason(&dir, "ax/done", "main").as_deref(), Some("already in origin/main"));
        assert!(merged_reason(&dir, "ax/new", "main").is_none() || std::process::Command::new("gh").arg("--version").output().is_ok());
        // merged into main locally, origin/main still behind: also counts
        g(&["checkout", "-q", "main"]);
        g(&["checkout", "-q", "-b", "ax/local"]);
        g(&commit);
        g(&["checkout", "-q", "main"]);
        g(&["merge", "-q", "--ff-only", "ax/local"]);
        assert_eq!(merged_reason(&dir, "ax/local", "main").as_deref(), Some("already in local main"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The remote deadline exists only while a worktree with commits of its own waits to be integrated.
    #[test]
    fn the_janitor_waits_for_github_only_while_work_is_unintegrated() {
        let root = std::env::temp_dir().join(format!("axl-await-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let root = git::canon(&root).unwrap();
        let repo = root.join("proj");
        std::fs::create_dir_all(&repo).unwrap();
        let g = |dir: &Path, args: &[&str]| assert!(git::run(dir, args).unwrap().ok, "git {args:?}");
        let commit = ["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "--allow-empty", "-m", "x"];
        g(&repo, &["init", "-q", "-b", "main"]);
        g(&repo, &commit);
        let store = root.join("projects.json");
        let p = projects::add(&store, &repo.to_string_lossy()).unwrap();
        let env = Env { busy: &|_| false, close: &|_| vec![], touched: &|_| false };
        let look = |p: &Project| {
            let mut waiting = false;
            sweep_project(p, &env, true, &mut vec![], &mut waiting).unwrap();
            waiting
        };
        // nothing yet
        assert!(!look(&p));
        // a fresh worktree with no commit of its own is not work waiting for anything
        let wt = projects::worktree_create(&p, "w").unwrap();
        assert!(!look(&p));
        // committed, not merged: waits (a PR could be merged on GitHub with no local event)
        g(Path::new(&wt.path), &commit);
        assert!(look(&p));
        // merged: it is for removal now, nothing to wait for
        g(&repo, &["merge", "-q", "--ff-only", "ax/w"]);
        assert!(!look(&p));
        let _ = git::remove_tree(&root);
    }

    #[test]
    fn sweep_removes_only_merged_idle_agent_worktrees() {
        let root = std::env::temp_dir().join(format!("axl-sweep-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        // git lists canonical paths (no 8.3 names on Windows, no /var -> /private/var on macOS)
        let root = git::canon(&root).unwrap();
        let repo = root.join("proj");
        std::fs::create_dir_all(&repo).unwrap();
        let g = |dir: &Path, args: &[&str]| assert!(git::run(dir, args).unwrap().ok, "git {args:?}");
        let commit = ["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "--allow-empty", "-m", "x"];
        g(&repo, &["init", "-q", "-b", "main"]);
        g(&repo, &commit);
        let store = root.join("projects.json");
        let p = projects::add(&store, &repo.to_string_lossy()).unwrap();
        // done: committed on its branch, then merged into main
        let done = projects::worktree_create(&p, "done").unwrap();
        g(Path::new(&done.path), &commit);
        g(&repo, &["merge", "-q", "--ff-only", "ax/done"]);
        // busy: merged too, but a session works in it
        let busy = projects::worktree_create(&p, "busy").unwrap();
        g(Path::new(&busy.path), &commit);
        g(&repo, &["merge", "-q", "--ff-only", "ax/busy"]);
        // dirty: merged, with an uncommitted file
        let dirty = projects::worktree_create(&p, "dirty").unwrap();
        g(Path::new(&dirty.path), &commit);
        g(&repo, &["merge", "-q", "--ff-only", "ax/dirty"]);
        std::fs::write(Path::new(&dirty.path).join("wip.txt"), "w").unwrap();
        // open: unmerged work
        let open = projects::worktree_create(&p, "open").unwrap();
        g(Path::new(&open.path), &commit);
        // fresh: nobody committed anything
        let fresh = projects::worktree_create(&p, "fresh").unwrap();

        let is_busy = |path: &str| axelstra_core::paths::same(path, &busy.path);
        // a dry run reports and removes nothing
        let dry = sweep(&store, &is_busy, true);
        assert_eq!(dry.len(), 1, "{dry:?}");
        assert_eq!(dry[0]["branch"], "ax/done");
        assert_eq!(dry[0]["dry_run"], true);
        assert_eq!(dry[0]["removed"], false);
        assert!(Path::new(&done.path).exists());

        let real = sweep(&store, &is_busy, false);
        assert_eq!(real.len(), 1, "{real:?}");
        assert_eq!(real[0]["removed"], true);
        assert!(!Path::new(&done.path).exists());
        for kept in [&busy, &dirty, &open, &fresh] {
            assert!(Path::new(&kept.path).exists(), "{} must stay", kept.path);
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A merged worktree whose ignored `node_modules` is deeper than the Windows path limit is still swept.
    #[test]
    fn sweep_removes_a_merged_worktree_with_a_deep_ignored_tree() {
        let (root, store, p) = repo_with_store("sweep-deep");
        let repo = root.join("proj");
        let g = |dir: &Path, args: &[&str]| assert!(git::run(dir, args).unwrap().ok, "git {args:?}");
        let done = projects::worktree_create(&p, "deep").unwrap();
        let wt = Path::new(&done.path);
        std::fs::write(
            wt.join(".gitignore"),
            "node_modules/
",
        )
        .unwrap();
        g(wt, &["add", ".gitignore"]);
        g(wt, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "-m", "ignore"]);
        g(&repo, &["merge", "-q", "--ff-only", "ax/deep"]);
        let mut d = git::verbatim(&wt.join("node_modules"));
        for i in 0..12 {
            d.push(format!("level-{i:02}-{}", "x".repeat(40)));
        }
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("leaf.js"), "x").unwrap();
        let real = sweep(&store, &|_| false, false);
        assert_eq!(real.len(), 1, "{real:?}");
        assert_eq!(real[0]["removed"], true, "{real:?}");
        assert!(!wt.exists());
        git::purge_trash(wt.parent().unwrap());
        let _ = git::remove_tree(&root);
    }

    fn repo_with_store(tag: &str) -> (std::path::PathBuf, std::path::PathBuf, Project) {
        let root = std::env::temp_dir().join(format!("axl-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let repo = root.join("proj");
        std::fs::create_dir_all(&repo).unwrap();
        let g = |args: &[&str]| assert!(git::run(&repo, args).unwrap().ok, "git {args:?}");
        g(&["init", "-q", "-b", "main"]);
        g(&["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "--allow-empty", "-m", "x"]);
        let store = root.join("projects.json");
        let p = projects::add(&store, &repo.to_string_lossy()).unwrap();
        (root, store, p)
    }

    /// Takes only the events of the repo under `root`: the queue is process-wide and tests run in parallel.
    fn take_events_under(root: &Path) -> Vec<Value> {
        // the temp dir may be spelled 8.3 here and long in the event: the unique folder name matches either way
        let tag = root.file_name();
        let mut q = EVENTS.lock().unwrap_or_else(|e| e.into_inner());
        let (mine, rest): (Vec<Value>, Vec<Value>) = std::mem::take(&mut *q)
            .into_iter()
            .partition(|e| e["path"].as_str().is_some_and(|p| Path::new(p).components().any(|c| Some(c.as_os_str()) == tag)));
        *q = rest;
        mine
    }

    const COMMIT: [&str; 8] = ["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "--allow-empty", "-m"];

    fn commit_in(dir: &Path, msg: &str) {
        let mut a = COMMIT.to_vec();
        a.push(msg);
        assert!(git::run(dir, &a).unwrap().ok);
    }

    #[test]
    fn a_stacked_branch_inside_another_live_branch_is_swept_but_the_top_one_stays() {
        let (root, store, p) = repo_with_store("stack");
        let base = projects::worktree_create(&p, "base").unwrap();
        commit_in(Path::new(&base.path), "base work");
        // the top branch grows from the base one and adds its own, unmerged commit
        let top = projects::create_at(&p, "top", "ax/base", true).unwrap().0;
        commit_in(Path::new(&top.path), "top work");
        let out = sweep(&store, &|_| false, false);
        assert_eq!(out.len(), 1, "{out:?}");
        assert_eq!(out[0]["branch"], "ax/base");
        assert_eq!(out[0]["into"], "ax/top");
        assert_eq!(out[0]["removed"], true);
        assert!(!Path::new(&base.path).exists());
        assert!(Path::new(&top.path).exists(), "the top branch holds unmerged work");
        // its commits are still reachable: the base commit is in the top branch
        assert!(git::run(Path::new(&p.path), &["rev-parse", "--verify", "ax/top"]).unwrap().ok);
        let ev = take_events_under(&root);
        assert!(ev.iter().any(|e| e["branch"] == "ax/base" && e["into"] == "ax/top"), "{ev:?}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn two_branches_at_the_same_commit_do_not_delete_each_other() {
        let (root, store, p) = repo_with_store("twins");
        let a = projects::worktree_create(&p, "a").unwrap();
        commit_in(Path::new(&a.path), "work");
        let b = projects::create_at(&p, "b", "ax/a", true).unwrap().0;
        // b has no commit of its own: same tip as a; only one of them may go, the work must stay on a branch
        let out = sweep(&store, &|_| false, false);
        let removed = out.iter().filter(|o| o["removed"] == true).count();
        assert!(removed <= 1, "{out:?}");
        let left = [&a, &b].iter().filter(|w| Path::new(&w.path).exists()).count();
        assert!(left >= 1);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_stacked_branch_is_kept_when_dirty_or_busy_or_when_the_other_branch_is_not_a_superset() {
        let (root, store, p) = repo_with_store("stackkeep");
        let base = projects::worktree_create(&p, "base").unwrap();
        commit_in(Path::new(&base.path), "base work");
        let top = projects::create_at(&p, "top", "ax/base", true).unwrap().0;
        commit_in(Path::new(&top.path), "top work");
        // unrelated branch with its own commit: not inside anything
        let solo = projects::worktree_create(&p, "solo").unwrap();
        commit_in(Path::new(&solo.path), "solo work");
        std::fs::write(Path::new(&base.path).join("wip.txt"), "w").unwrap();
        let out = sweep(&store, &|_| false, false);
        assert!(out.is_empty(), "dirty base and unmerged solo/top must stay: {out:?}");
        std::fs::remove_file(Path::new(&base.path).join("wip.txt")).unwrap();
        let busy_base = base.path.clone();
        let out = sweep(&store, &|path| axelstra_core::paths::same(path, &busy_base), false);
        assert!(out.is_empty(), "busy base must stay: {out:?}");
        for w in [&base, &top, &solo] {
            assert!(Path::new(&w.path).exists());
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A child process that has the worktree as its cwd (what a Claude session left alive does).
    fn holder(dir: &str) -> std::process::Child {
        let (prog, args): (&str, &[&str]) = if cfg!(windows) { ("ping", &["-n", "60", "127.0.0.1"]) } else { ("sleep", &["60"]) };
        Command::new(prog)
            .args(args)
            .current_dir(dir)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap()
    }

    #[cfg(windows)]
    fn open_without_delete_share(dir: &str) -> std::fs::File {
        use std::os::windows::fs::OpenOptionsExt;
        // FILE_SHARE_READ | FILE_SHARE_WRITE, no FILE_SHARE_DELETE
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .share_mode(3)
            .open(Path::new(dir).join("held.lock"))
            .unwrap()
    }

    #[cfg(not(windows))]
    fn open_without_delete_share(dir: &str) -> std::fs::File {
        std::fs::File::create(Path::new(dir).join("held.lock")).unwrap()
    }

    fn merged_worktree(tag: &str, name: &str) -> (std::path::PathBuf, std::path::PathBuf, Project, git::Worktree) {
        let (root, store, p) = repo_with_store(tag);
        let wt = projects::worktree_create(&p, name).unwrap();
        commit_in(Path::new(&wt.path), "work");
        assert!(git::run(Path::new(&p.path), &["merge", "-q", "--ff-only", &format!("ax/{name}")]).unwrap().ok);
        (root, store, p, wt)
    }

    fn branch_exists(p: &Project, branch: &str) -> bool {
        git::run(Path::new(&p.path), &["rev-parse", "--verify", branch]).unwrap().ok
    }

    #[test]
    fn a_folder_held_by_a_process_is_left_whole_and_goes_once_the_session_is_closed() {
        let (root, store, p, wt) = merged_worktree("held", "held");
        let child = std::cell::RefCell::new(holder(&wt.path));
        // nothing closes the session: on Windows the folder cannot move, so nothing at all may change
        let out = sweep(&store, &|_| false, false);
        if cfg!(windows) {
            assert_eq!(out[0]["removed"], false, "{out:?}");
            assert_eq!(out[0]["pending"], true, "{out:?}");
            assert!(Path::new(&wt.path).exists());
            assert!(branch_exists(&p, "ax/held"), "the branch must not go while the folder stays");
            assert!(git::worktree_list(Path::new(&p.path)).unwrap().iter().any(|w| axelstra_core::paths::same(&w.path, &wt.path)));
            // the sweep closes the session first, waits for it, and then the removal goes through
            let killed = std::cell::Cell::new(false);
            let close = |_: &str| {
                let _ = child.borrow_mut().kill();
                let _ = child.borrow_mut().wait();
                killed.set(true);
                vec!["Tarefa 3".to_string()]
            };
            let env = Env { busy: &|_| false, close: &close, touched: &|_| false };
            let out = sweep_with(&store, &env, false);
            assert!(killed.get());
            assert_eq!(out[0]["removed"], true, "{out:?}");
            assert_eq!(out[0]["closed"][0], "Tarefa 3");
        } else {
            let _ = child.borrow_mut().kill();
            let _ = child.borrow_mut().wait();
            assert_eq!(out[0]["removed"], true, "{out:?}");
        }
        assert!(!Path::new(&wt.path).exists());
        assert!(!branch_exists(&p, "ax/held"));
        let ev = take_events_under(&root);
        if cfg!(windows) {
            assert!(ev.iter().any(|e| e["code"] == "janitor.removed_closed" && e["params"]["sessions"] == "Tarefa 3"), "{ev:?}");
        }
        git::purge_trash(wt_parent(&wt));
        let _ = git::remove_tree(&root);
    }

    fn wt_parent(wt: &git::Worktree) -> &Path {
        Path::new(&wt.path).parent().unwrap()
    }

    #[test]
    fn strict_removal_changes_nothing_when_the_folder_is_held() {
        let (root, _store, p, wt) = merged_worktree("strict", "strict");
        let mut child = holder(&wt.path);
        // a file open without delete sharing inside the folder: Windows refuses to move the folder whatever its
        // version does with a bare working directory (Server 2025 lets that one be renamed)
        let _open = open_without_delete_share(&wt.path);
        let r = projects::worktree_remove_strict(&p, &wt.path);
        if cfg!(windows) {
            let e = r.as_ref().unwrap_err();
            assert_eq!(axelstra_core::msg::find(e).map(|c| c.code.as_str()), Some("worktree.folder_busy"));
            assert!(Path::new(&wt.path).join(".git").exists());
            assert!(branch_exists(&p, "ax/strict"));
        }
        let _ = child.kill();
        let _ = child.wait();
        drop(_open);
        if cfg!(windows) {
            projects::worktree_remove_strict(&p, &wt.path).unwrap();
        } else {
            // an open handle does not hold a folder here: the first call already removed it
            r.unwrap();
        }
        assert!(!Path::new(&wt.path).exists());
        assert!(!branch_exists(&p, "ax/strict"));
        git::purge_trash(wt_parent(&wt));
        let _ = git::remove_tree(&root);
    }

    #[test]
    fn an_unmarked_empty_worktree_is_never_removed_even_when_old_or_reported_done() {
        let (root, store, p) = repo_with_store("unmarked");
        let old = projects::worktree_create(&p, "legacy").unwrap();
        let hand = projects::worktree_create(&p, "hand").unwrap();
        projects::set_manual(&store, &p.id, &hand.path).unwrap();
        // the reported-done signal and a quiet folder must not matter without the `auto` mark
        integrate::mark_done(&old.path, true);
        integrate::mark_done(&hand.path, true);
        let out = sweep(&store, &|_| false, false);
        assert!(out.iter().all(|o| o["removed"] != true), "{out:?}");
        // migration marks it manual and changes nothing else
        projects::migrate_marks(&store).unwrap();
        let p2 = projects::find(&store, &p.id).unwrap();
        assert!(p2.manual.contains(&axelstra_core::paths::norm(&old.path)) && p2.auto.is_empty());
        let out = sweep(&store, &|_| false, false);
        assert!(out.iter().all(|o| o["removed"] != true), "{out:?}");
        assert!(Path::new(&old.path).exists() && Path::new(&hand.path).exists());
        let _ = git::remove_tree(&root);
    }

    #[test]
    fn migration_runs_for_existing_worktrees_only_and_keeps_auto() {
        let (root, store, p) = repo_with_store("migrate");
        let auto = projects::worktree_create(&p, "robot").unwrap();
        projects::set_auto(&store, &p.id, &auto.path).unwrap();
        let old = projects::worktree_create(&p, "old").unwrap();
        projects::migrate_marks(&store).unwrap();
        // created after the migration: no mark yet, and the migration does not run again by itself
        let late = projects::worktree_create(&p, "late").unwrap();
        let p2 = projects::find(&store, &p.id).unwrap();
        assert!(p2.auto.contains(&axelstra_core::paths::norm(&auto.path)) && !p2.manual.contains(&axelstra_core::paths::norm(&auto.path)));
        assert!(p2.manual.contains(&axelstra_core::paths::norm(&old.path)));
        assert!(!p2.manual.contains(&axelstra_core::paths::norm(&late.path)) && !p2.auto.contains(&axelstra_core::paths::norm(&late.path)));
        let _ = git::remove_tree(&root);
    }

    #[test]
    fn an_empty_auto_worktree_goes_only_once_old_or_reported_and_a_fresh_one_stays() {
        let (root, store, p) = repo_with_store("autoempty");
        let fresh = projects::worktree_create(&p, "fresh").unwrap();
        let done = projects::worktree_create(&p, "done").unwrap();
        for w in [&fresh, &done] {
            projects::set_auto(&store, &p.id, &w.path).unwrap();
        }
        integrate::mark_done(&done.path, true);
        let out = sweep(&store, &|_| false, false);
        assert_eq!(out.len(), 1, "{out:?}");
        assert_eq!(out[0]["branch"], "ax/done");
        assert_eq!(out[0]["removed"], true);
        assert!(Path::new(&fresh.path).exists(), "an auto worktree younger than 10 min with no report stays");
        git::purge_trash(wt_parent(&done));
        let _ = git::remove_tree(&root);
    }

    #[test]
    fn an_auto_integrated_worktree_is_removed_but_not_while_touched() {
        let (root, store, p, wt) = merged_worktree("autointeg", "robot");
        projects::set_auto(&store, &p.id, &wt.path).unwrap();
        let touched = std::cell::Cell::new(true);
        let t = |_: &str| touched.get();
        let env = Env { busy: &|_| false, close: &|_| vec![], touched: &t };
        assert!(sweep_with(&store, &env, false).is_empty());
        touched.set(false);
        let out = sweep_with(&store, &env, false);
        assert_eq!(out.len(), 1, "{out:?}");
        assert_eq!(out[0]["removed"], true);
        assert!(!Path::new(&wt.path).exists());
        git::purge_trash(wt_parent(&wt));
        let _ = git::remove_tree(&root);
    }

    #[test]
    fn a_branch_at_the_default_tip_without_commits_is_never_merged() {
        let (root, _store, p) = repo_with_store("tiptip");
        let w = projects::worktree_create(&p, "same").unwrap();
        assert!(merged_reason(Path::new(&p.path), "ax/same", "main").is_none());
        git::purge_trash(wt_parent(&w));
        let _ = git::remove_tree(&root);
    }

    #[test]
    fn a_worktree_made_by_hand_stays_while_touched_and_is_never_swept_as_empty() {
        let (root, store, p, wt) = merged_worktree("manual", "mine");
        let empty = projects::worktree_create(&p, "blank").unwrap();
        for w in [&wt, &empty] {
            projects::set_manual(&store, &p.id, &w.path).unwrap();
        }
        // an empty one reported `done`: for an agent's worktree that makes it removable at once, not for a hand-made one
        integrate::mark_done(&empty.path, true);
        let touched = std::cell::Cell::new(true);
        let t = |_: &str| touched.get();
        let env = Env { busy: &|_| false, close: &|_| vec![], touched: &t };
        let out = sweep_with(&store, &env, false);
        assert!(out.iter().all(|o| o["removed"] != true), "{out:?}");
        assert!(Path::new(&wt.path).exists() && Path::new(&empty.path).exists());
        // nobody used it for a while and its branch is really integrated: it goes; the empty one still does not
        touched.set(false);
        let out = sweep_with(&store, &env, false);
        assert_eq!(out.len(), 1, "{out:?}");
        assert_eq!(out[0]["branch"], "ax/mine");
        assert_eq!(out[0]["removed"], true);
        assert!(Path::new(&empty.path).exists(), "an empty hand-made worktree is kept");
        git::purge_trash(wt_parent(&wt));
        let _ = git::remove_tree(&root);
    }

    #[test]
    fn an_empty_agent_worktree_that_reported_done_is_still_swept() {
        let (root, store, p) = repo_with_store("emptyagent");
        let empty = projects::worktree_create(&p, "agentempty").unwrap();
        projects::set_auto(&store, &p.id, &empty.path).unwrap();
        integrate::mark_done(&empty.path, true);
        let out = sweep(&store, &|_| false, false);
        assert!(out.iter().any(|o| o["why"] == "empty" && o["removed"] == true), "{out:?}");
        git::purge_trash(wt_parent(&empty));
        let _ = git::remove_tree(&root);
    }
    #[test]
    fn pending_retry_backs_off_and_is_bounded() {
        let d: Vec<u64> = (1..=9).map(|n| retry_delay(n).as_secs()).collect();
        assert_eq!(d, [60, 120, 240, 480, 960, 1800, 1800, 1800, 1800]);
        assert_eq!(retry_delay(0).as_secs(), 60);
        assert_eq!(retry_delay(u32::MAX).as_secs(), 1800);
    }

    /// The retry is by event: once the holder is gone, the next sweep (what its exit kicks) removes the worktree.
    #[test]
    fn a_pending_removal_is_retried_by_the_next_sweep_once_the_holder_exits() {
        let (root, store, p, wt) = merged_worktree("retry", "retry");
        projects::set_auto(&store, &p.id, &wt.path).unwrap();
        let mut child = holder(&wt.path);
        let out = sweep(&store, &|_| false, false);
        if cfg!(windows) {
            assert_eq!(out[0]["pending"], true, "{out:?}");
            assert!(Path::new(&wt.path).exists());
        }
        let _ = child.kill();
        let _ = child.wait();
        let out = sweep(&store, &|_| false, false);
        if cfg!(windows) {
            assert_eq!(out[0]["removed"], true, "{out:?}");
        }
        assert!(!Path::new(&wt.path).exists());
        git::purge_trash(wt_parent(&wt));
        let _ = git::remove_tree(&root);
    }

    /// A worktree whose branch gets `files` committed one per commit: (worktree, branch).
    fn worktree_with_files(p: &Project, name: &str, files: &[&str]) -> (git::Worktree, String) {
        let wt = projects::worktree_create(p, name).unwrap();
        for f in files {
            std::fs::write(Path::new(&wt.path).join(f), f).unwrap();
            assert!(git::run(Path::new(&wt.path), &["add", f]).unwrap().ok);
            assert!(git::run(Path::new(&wt.path), &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "-m", f]).unwrap().ok);
        }
        (wt, format!("ax/{name}"))
    }

    fn on_main(p: &Project, args: &[&str]) {
        let mut a = vec!["-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false"];
        a.extend_from_slice(args);
        assert!(git::run(Path::new(&p.path), &a).unwrap().ok, "git {args:?}");
    }

    /// Integration with no ancestor: a squash merge and a full cherry-pick both count, a partial cherry-pick and work
    /// nobody integrated do not.
    #[test]
    fn squash_merged_and_cherry_picked_worktrees_go_and_partial_or_unintegrated_ones_stay() {
        let (root, store, p) = repo_with_store("equiv");
        let (sq, sq_b) = worktree_with_files(&p, "sq", &["s1.txt", "s2.txt"]);
        let (cp, cp_b) = worktree_with_files(&p, "cp", &["c1.txt", "c2.txt"]);
        let (part, part_b) = worktree_with_files(&p, "part", &["p1.txt", "p2.txt"]);
        let (open, _) = worktree_with_files(&p, "open", &["o1.txt"]);
        on_main(&p, &["merge", "--squash", &sq_b]);
        on_main(&p, &["commit", "-qm", "squashed"]);
        on_main(&p, &["cherry-pick", &format!("{cp_b}~1"), &cp_b]);
        on_main(&p, &["cherry-pick", &format!("{part_b}~1")]);
        let out = sweep(&store, &|_| false, false);
        let removed: Vec<&str> = out.iter().filter(|o| o["removed"] == true).filter_map(|o| o["branch"].as_str()).collect();
        assert!(removed.contains(&sq_b.as_str()) && removed.contains(&cp_b.as_str()) && removed.len() == 2, "{out:?}");
        assert!(!Path::new(&sq.path).exists() && !Path::new(&cp.path).exists());
        assert!(Path::new(&part.path).exists() && Path::new(&open.path).exists(), "{out:?}");
        assert!(branch_exists(&p, &part_b), "a partly picked branch keeps its work");
        git::purge_trash(wt_parent(&sq));
        let _ = git::remove_tree(&root);
    }

    /// An orchestrator integration branch (it only receives `merge --no-ff`) goes once merged into the default branch.
    #[test]
    fn a_merged_orch_integration_branch_goes_and_an_unmerged_one_stays() {
        let (root, store, p) = repo_with_store("orchint");
        let mk = |run: &str, merge: bool| {
            let (_task, task_b) = worktree_with_files(&p, &format!("{run}-card"), &[&format!("{run}.txt")]);
            let orch = format!("orch/{run}");
            let integ = projects::create_at(&p, &format!("{run}-integ"), "main", true).unwrap().0;
            let ip = Path::new(&integ.path);
            assert!(git::run(ip, &["checkout", "-q", "-b", &orch]).unwrap().ok);
            let merge_card = ["-c", "user.name=t", "-c", "user.email=t@t", "merge", "-q", "--no-ff", "-m", "card", task_b.as_str()];
            assert!(git::run(ip, &merge_card).unwrap().ok);
            assert!(has_own_commits(Path::new(&p.path), &orch), "a merge counts as an own commit of orch/*");
            if merge {
                on_main(&p, &["merge", "-q", "--no-ff", "-m", "run", &orch]);
            }
            integ
        };
        let done = mk("r1", true);
        let open = mk("r2", false);
        let out = sweep(&store, &|_| false, false);
        assert!(out.iter().any(|o| o["branch"] == "orch/r1" && o["removed"] == true), "{out:?}");
        assert!(!out.iter().any(|o| o["branch"] == "orch/r2" && o["removed"] == true), "{out:?}");
        assert!(!Path::new(&done.path).exists() && Path::new(&open.path).exists());
        git::purge_trash(wt_parent(&done));
        let _ = git::remove_tree(&root);
    }

    #[test]
    fn a_pending_event_names_the_holder_or_the_reason() {
        let p = Project { id: "p".into(), ..Default::default() };
        let path = "/w";
        let err = axelstra_core::coded!("worktree.folder_busy", "the folder {path} is still in use", path = path);
        let e = pending_event(&p, "/w", "ax/x", &err, Some(("node".into(), 42)));
        assert_eq!((e["kind"].as_str(), e["code"].as_str()), (Some("pending"), Some("janitor.pending")));
        assert_eq!((e["holder"]["name"].as_str(), e["holder"]["pid"].as_u64()), (Some("node"), Some(42)));
        assert_eq!((e["params"]["branch"].as_str(), e["params"]["pid"].as_str()), (Some("x"), Some("42")));
        assert_eq!(e["reason"], "worktree.folder_busy");
        let e = pending_event(&p, "/w", "ax/x", &err, None);
        assert_eq!(e["code"], "janitor.pending_busy");
        assert!(e["holder"].is_null() && e["params"]["detail"].as_str().unwrap().contains("still in use"), "{e}");
        // one report per episode
        let k = format!("/tmp/axl-episode-{}", std::process::id());
        assert!(pending_begin(&k) && !pending_begin(&k));
        pending_end(&k);
        assert!(pending_begin(&k));
        pending_end(&k);
    }

    /// The holder of a folder is found: on Linux by working directory, on Windows by an executable that runs from it.
    #[cfg(any(windows, target_os = "linux"))]
    #[test]
    fn the_process_holding_a_folder_is_found() {
        let (root, _store, p) = repo_with_store("holder");
        let wt = projects::worktree_create(&p, "h").unwrap();
        assert!(holder_of(Path::new(&wt.path)).is_none());
        let mut child = if cfg!(windows) {
            let exe = Path::new(&wt.path).join("axl-hold.exe");
            std::fs::copy(r"C:\Windows\System32\PING.EXE", &exe).unwrap();
            Command::new(&exe).args(["-n", "60", "127.0.0.1"]).stdout(std::process::Stdio::null()).spawn().unwrap()
        } else {
            holder(&wt.path)
        };
        let found = holder_of(Path::new(&wt.path));
        let _ = child.kill();
        let _ = child.wait();
        let (name, pid) = found.expect("holder");
        assert_eq!(pid, child.id());
        assert!(name.to_lowercase().contains(if cfg!(windows) { "axl-hold" } else { "sleep" }), "{name}");
        let _ = git::remove_tree(&root);
    }

    /// A held integrated worktree is reported once with its holder, and the retries say nothing new.
    #[cfg(windows)]
    #[test]
    fn a_held_integrated_worktree_is_reported_pending_once() {
        let (root, store, p, wt) = merged_worktree("pendev", "pendev");
        // ignored, so the worktree is still clean (a dirty one is never removed)
        std::fs::create_dir_all(Path::new(&p.path).join(".git/info")).unwrap();
        std::fs::write(Path::new(&p.path).join(".git/info/exclude"), "axl-pend.exe\nheld.lock\n").unwrap();
        let exe = Path::new(&wt.path).join("axl-pend.exe");
        std::fs::copy(r"C:\Windows\System32\PING.EXE", &exe).unwrap();
        let mut child =
            Command::new(&exe).args(["-n", "60", "127.0.0.1"]).current_dir(&wt.path).stdout(std::process::Stdio::null()).spawn().unwrap();
        let open = open_without_delete_share(&wt.path);
        let out = sweep(&store, &|_| false, false);
        assert_eq!(out[0]["pending"], true, "{out:?}");
        let ev = take_events_under(&root);
        let pend: Vec<&Value> = ev.iter().filter(|e| e["kind"] == "pending").collect();
        assert_eq!(pend.len(), 1, "{ev:?}");
        assert_eq!(pend[0]["code"], "janitor.pending");
        assert_eq!(pend[0]["holder"]["pid"].as_u64(), Some(child.id() as u64), "{ev:?}");
        let _ = sweep(&store, &|_| false, false);
        assert!(take_events_under(&root).iter().all(|e| e["kind"] != "pending"), "a retry does not report again");
        let _ = child.kill();
        let _ = child.wait();
        drop(open);
        let out = sweep(&store, &|_| false, false);
        assert_eq!(out[0]["removed"], true, "{out:?}");
        git::purge_trash(wt_parent(&wt));
        let _ = git::remove_tree(&root);
    }

    #[test]
    fn integrated_into_names_the_target() {
        assert_eq!(integrated_into("already in origin/main", "main"), "main");
        assert_eq!(integrated_into("already in local main", "main"), "main");
        assert_eq!(integrated_into("already in ax/top", "main"), "ax/top");
        assert_eq!(integrated_into("PR #4 merged", "main"), "main");
    }
}
