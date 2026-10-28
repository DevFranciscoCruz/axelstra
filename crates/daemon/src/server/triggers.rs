//! `triggers.*`: the CI-fix trigger of a project (see `crate::triggers`) and the loop that runs it. The same loop keeps
//! the pull requests of the worktrees the app or the trigger looks at fresh and says `pr.changed` when one changed (see
//! `crate::prwatch`): it runs on events, plus one computed deadline, never on a fixed period.
use super::*;
use crate::prwatch::{self, Observed};
use crate::triggers as trig;
use axelstra_core::paths;

/// A PR whose answer differs from the last one the daemon had.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Change {
    pub(super) worktree: String,
    pub(super) number: u64,
    pub(super) state: String,
    pub(super) ci: String,
}

/// Tells the app (`pr.changed`) and, when a PR is no longer open, the janitor (the branch may be removable now).
pub(super) fn emit(reg: &Registry, changes: &[Change]) {
    for c in changes {
        reg.broadcast_event(json!({"event": "pr.changed", "worktree": c.worktree, "number": c.number, "state": c.state, "ci": c.ci}));
        if !c.state.is_empty() && c.state != "open" {
            reg.janitor.bump();
        }
    }
}

/// Records a look the app made itself (`git.checks`) and tells everybody (not only that panel) when it found news.
pub(super) fn seen_by_app(reg: &Registry, wt: &std::path::Path, st: &crate::checks::Status) {
    if prwatch::note_seen(wt, st) == Observed::Changed {
        emit(reg, &[change_of(wt, st)]);
    }
}

fn change_of(wt: &std::path::Path, st: &crate::checks::Status) -> Change {
    Change { worktree: wt.to_string_lossy().into_owned(), number: st.number, state: st.pr_state.clone(), ci: st.ci.clone() }
}

/// `Env` of the running daemon: real `gh`, real sessions. `event` says why this pass runs: an event re-reads every PR whose
/// last look is old, the deadline only those that are due.
struct DaemonEnv {
    reg: Arc<Registry>,
    event: bool,
    changes: Mutex<Vec<Change>>,
}

impl DaemonEnv {
    fn new(reg: Arc<Registry>, event: bool) -> Self {
        DaemonEnv { reg, event, changes: Mutex::new(vec![]) }
    }

    /// Asks GitHub and records the answer; a different one is queued for `pr.changed`.
    fn look(&self, wt: &std::path::Path) -> crate::checks::Status {
        let st = crate::checks::status(wt);
        if prwatch::TABLE.lock_recover().note(wt, &st, prwatch::now_ms()) == Observed::Changed {
            self.changes.lock_recover().push(change_of(wt, &st));
        }
        st
    }

    fn take(&self) -> Vec<Change> {
        std::mem::take(&mut *self.changes.lock_recover())
    }
}

impl trig::Env for DaemonEnv {
    fn now(&self) -> u64 {
        u64::try_from(crate::automations::now_ts()).unwrap_or(0)
    }

    fn projects(&self) -> Vec<crate::projects::Project> {
        crate::projects::load(&self.reg.store)
    }

    fn worktrees(&self, p: &crate::projects::Project) -> Vec<axelstra_core::git::Worktree> {
        axelstra_core::git::worktree_list(std::path::Path::new(&p.path)).unwrap_or_default()
    }

    fn live(&self, project: &str, worktree: &str) -> trig::Live {
        let want = paths::norm(worktree);
        let sessions = self.reg.sessions.lock_recover();
        let metas = self.reg.metas.lock_recover();
        let mut live = trig::Live::default();
        for (id, m) in metas.iter() {
            let here = m["kind"] == "agent"
                && m["project"].as_str() == Some(project)
                && paths::norm(m["worktree"].as_str().unwrap_or_default()) == want
                && sessions.get(id).is_some_and(|s| s.try_wait().is_none());
            if here {
                live.any = true;
                live.fix_running |= m["trigger"] == "ci_fix";
            }
        }
        live
    }

    fn checks(&self, worktree: &std::path::Path) -> crate::checks::Status {
        // the last look counts while it is young (event) or not due yet (deadline); otherwise ask GitHub
        let cached = prwatch::TABLE.lock_recover().fresh(worktree, prwatch::now_ms(), self.event);
        cached.unwrap_or_else(|| self.look(worktree))
    }

    fn spawn_fix(&self, p: &crate::projects::Project, worktree: &str, prompt: &str) -> Result<u32> {
        // AXELSTRA_AGENT_CLI swaps the agent binary (a wrapper script, or a harmless stand-in in tests)
        let (program, args) = match std::env::var("AXELSTRA_AGENT_CLI") {
            Ok(cli) => crate::automations::raw_command(&cli, prompt),
            Err(_) => {
                let ag = axelstra_core::agents::require(trig::AGENT)?;
                if ag.locate().is_none() {
                    return Err(ag.missing_error());
                }
                crate::automations::agent_command(trig::AGENT, prompt, axelstra_core::git::folder_trusted(std::path::Path::new(worktree)))?
            }
        };
        // a task of the worktree like any other: `session.spawn` names it and makes the lead its parent. The agent
        // runs with its default permissions: nothing here asks for a bypass mode.
        let meta = json!({"project": p.id, "worktree": worktree, "kind": "agent", "agent": trig::AGENT, "trigger": "ci_fix"});
        let mut client = crate::client::Client::connect(&self.reg.pipe, &self.reg.token)?;
        let r = client.call(
            "session.spawn",
            json!({"program": program, "args": args, "cwd": worktree, "label": trig::AGENT, "meta": meta,
                   "goal": "Corrigir o CI (disparado automaticamente)", "cols": 120, "rows": 30}),
        )?;
        r["id"].as_u64().and_then(|i| u32::try_from(i).ok()).ok_or_else(|| axelstra_core::coded!("daemon.no_id", "daemon returned no id"))
    }
}

/// The agents running now, as (project, worktree): when this changes a worktree may have got (or lost) the agent the
/// trigger needs.
fn agent_signature(reg: &Registry) -> Vec<(String, String)> {
    let sessions = reg.sessions.lock_recover();
    let metas = reg.metas.lock_recover();
    let mut sig: Vec<(String, String)> = metas
        .iter()
        .filter(|(id, m)| m["kind"] == "agent" && sessions.get(id).is_some_and(|s| s.try_wait().is_none()))
        .map(|(_, m)| (m["project"].as_str().unwrap_or_default().to_string(), paths::norm(m["worktree"].as_str().unwrap_or_default())))
        .collect();
    sig.sort();
    sig
}

/// One pass: the trigger over the opted-in projects, then the worktrees of interest whose last look no longer counts.
fn pass(reg: &Arc<Registry>, seen: &mut std::collections::HashSet<String>, event: bool) {
    let env = DaemonEnv::new(reg.clone(), event);
    let path = trig::store_path(&reg.store);
    let fired = trig::tick(&env, &path, &reg.audit, seen);
    for o in &fired {
        eprintln!("triggers: {} {} {}: {}", o.kind, o.project, o.worktree, o.detail);
    }
    if !fired.is_empty() {
        // sessions changed: watchers refresh
        reg.wt_gen.fetch_add(1, Ordering::SeqCst);
    }
    let now = prwatch::now_ms();
    let stale = prwatch::TABLE.lock_recover().stale(now, event);
    for wt in stale {
        env.look(&wt);
    }
    prwatch::TABLE.lock_recover().prune(now);
    emit(reg, &env.take());
    reg.announce_events();
}

/// Keeps the CI trigger and the PRs of interest current without a fixed period. It runs when something that usually comes
/// with a change happened: a ref moved in a project's repository (fetch, push, commit), `done`, a merge or a session
/// ending (the janitor's wake-ups), an agent started or stopped, the window got focus (`gh.pr_refresh`), or the app
/// looked at a PR. Between events there is at most one timer: the earliest moment a known PR is due a new look (a minute
/// while its CI runs, a quarter of an hour while it is open), and none when no PR is due. A project that did not opt in
/// costs one small file read.
pub(super) fn spawn_loop(reg: Arc<Registry>) {
    // done, merge, session exit, flow end...: whatever wakes the janitor can also change a PR or the worktree it is in
    let r = reg.clone();
    std::thread::spawn(move || {
        let mut seen = r.janitor.now();
        loop {
            seen = r.janitor.wait(seen, None);
            prwatch::WAKE.bump();
        }
    });
    // an agent started or stopped: only the set of running agents matters, not every pulse
    let r = reg.clone();
    std::thread::spawn(move || {
        let mut seen = r.pulse.now();
        let mut last = agent_signature(&r);
        loop {
            seen = r.pulse.wait(seen, None);
            let now = agent_signature(&r);
            if now != last {
                last = now;
                prwatch::WAKE.bump();
            }
        }
    });
    std::thread::spawn(move || {
        let mut seen_refusals = std::collections::HashSet::new();
        let r = reg.clone();
        // a remote ref moved: a merge may have arrived, so the janitor looks too (it has its own gap)
        let mut refs = prwatch::RefWatch::new(Arc::new(move || r.janitor.bump()));
        let mut event = true;
        loop {
            if reg.closing.load(Ordering::SeqCst) {
                return;
            }
            // taken before the pass: a wake-up during it makes the next wait return at once
            let seen = prwatch::WAKE.now();
            let repos: Vec<std::path::PathBuf> =
                crate::projects::load(&reg.store).iter().map(|p| std::path::PathBuf::from(&p.path)).collect();
            refs.sync(&repos);
            pass(&reg, &mut seen_refusals, event);
            let deadline = prwatch::TABLE.lock_recover().next_deadline(prwatch::now_ms()).map(Duration::from_millis);
            event = prwatch::WAKE.wait(seen, deadline) != seen;
            if event {
                // a ref update or a fetch is several writes: one pass for the burst
                std::thread::sleep(Duration::from_millis(250));
            }
        }
    });
}

fn limits(t: &trig::ProjectTriggers) -> Value {
    json!({"cooldown_secs": t.cooldown_secs, "max_per_hour": t.max_per_hour, "poll_secs": prwatch::PENDING_MS / 1000,
           "cooldown_range": [trig::COOLDOWN_RANGE.0, trig::COOLDOWN_RANGE.1],
           "max_per_hour_range": [trig::MAX_PER_HOUR_RANGE.0, trig::MAX_PER_HOUR_RANGE.1]})
}

fn state(project: &str, t: &trig::ProjectTriggers) -> Value {
    json!({
        "project": project, "ci_fix": t.ci_fix, "confirmed_at": t.confirmed_at,
        "limits": limits(t), "history": t.history, "last": t.last,
        "agent": trig::AGENT, "confirm_word": trig::CONFIRM_WORD,
        "template": trig::PROMPT_TEMPLATE, "example": trig::example_prompt(),
    })
}

pub(super) fn call(c: &mut Ctx) -> Result<Value> {
    let (req, p, reg) = (c.req, c.p, c.reg);
    if req.method == "triggers.events" {
        return Ok(json!({"events": trig::take_events()}));
    }
    let pr = crate::projects::find(&reg.store, p["project"].as_str().unwrap_or_default())?;
    let path = trig::store_path(&reg.store);
    match req.method.as_str() {
        // the state of the project, the exact prompt template and the limits; readable by everyone
        "triggers.get" => Ok(state(&pr.id, &trig::load(&path).get(&pr.id).cloned().unwrap_or_default())),
        // what enabling does, for the confirmation dialog: the prompt filled with an example failure and the limits
        "triggers.preview" => {
            let t = trig::load(&path).get(&pr.id).cloned().unwrap_or_default();
            Ok(json!({"project": pr.id, "prompt": trig::example_prompt(), "template": trig::PROMPT_TEMPLATE,
                      "limits": limits(&t), "agent": trig::AGENT, "confirm_word": trig::CONFIRM_WORD,
                      "never_main": true, "never_pr_worktrees": true, "audit_method": trig::AUDIT_METHOD}))
        }
        "triggers.set" => {
            let _g = trig::STORE_LOCK.lock_recover();
            let mut all = trig::load(&path);
            let t = all.entry(pr.id.clone()).or_default();
            let int = |k: &str, lo: u64, hi: u64| -> Result<Option<u64>> {
                match p.get(k) {
                    None | Some(Value::Null) => Ok(None),
                    Some(v) => match v.as_u64().filter(|n| (lo..=hi).contains(n)) {
                        Some(n) => Ok(Some(n)),
                        None => Err(axelstra_core::coded!(
                            "triggers.bad_number",
                            "{k} must be a whole number from {lo} to {hi}",
                            k = k,
                            lo = lo,
                            hi = hi
                        )),
                    },
                }
            };
            let cooldown = int("cooldown_secs", trig::COOLDOWN_RANGE.0, trig::COOLDOWN_RANGE.1)?;
            let max = int("max_per_hour", u64::from(trig::MAX_PER_HOUR_RANGE.0), u64::from(trig::MAX_PER_HOUR_RANGE.1))?;
            let want = match p.get("ci_fix") {
                None | Some(Value::Null) => None,
                Some(v) => Some(v.as_bool().ok_or_else(|| axelstra_core::coded!("triggers.bad_ci_fix", "ci_fix must be true or false"))?),
            };
            // the first enabling is something the user typed, not a flag
            if want == Some(true) && t.confirmed_at.is_none() {
                if p["confirm"].as_str() != Some(trig::CONFIRM_WORD) {
                    return Err(axelstra_core::coded!(
                        "triggers.confirm_needed",
                        "enabling needs confirm = \"{word}\" the first time",
                        word = trig::CONFIRM_WORD
                    ));
                }
                t.confirmed_at = Some(u64::try_from(crate::automations::now_ts()).unwrap_or(0));
            }
            if let Some(b) = want {
                t.ci_fix = b;
            }
            if let Some(n) = cooldown {
                t.cooldown_secs = n;
            }
            if let Some(n) = max {
                t.max_per_hour = n as u32;
            }
            let out = state(&pr.id, t);
            trig::save(&path, &all)?;
            Ok(out)
        }
        m => Err(unknown_method(m)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checks::Status;
    use std::sync::atomic::AtomicBool;

    fn st(state: &str, ci: &str) -> Status {
        Status { number: 3, pr_state: state.into(), ci: ci.into(), head_sha: "abc".into(), ..Default::default() }
    }

    fn registry() -> (Arc<Registry>, mpsc::Receiver<Vec<u8>>) {
        let dir = std::env::temp_dir().join(format!("axl-prchanged-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let reg = Arc::new(Registry::new(dir.join("projects.json"), "p".into(), "t".into()));
        let (tx, rx) = mpsc::channel::<Vec<u8>>(16);
        reg.watchers.lock_recover().push((Arc::new(AtomicBool::new(false)), tx));
        (reg, rx)
    }

    fn frames(rx: &mut mpsc::Receiver<Vec<u8>>) -> Vec<Value> {
        let mut out = vec![];
        while let Ok(f) = rx.try_recv() {
            out.push(serde_json::from_slice(&f[5..]).unwrap());
        }
        out
    }

    /// The app's own look only says `pr.changed` when it differs from what the daemon had, never for the first one.
    #[test]
    fn pr_changed_is_sent_when_the_answer_differs_and_only_then() {
        let (reg, mut rx) = registry();
        let wt = std::path::PathBuf::from("C:/axl-prchanged/wt-a");
        seen_by_app(&reg, &wt, &st("open", "pending"));
        assert!(frames(&mut rx).is_empty(), "the first look is nobody's news");
        seen_by_app(&reg, &wt, &st("open", "pending"));
        assert!(frames(&mut rx).is_empty());
        seen_by_app(&reg, &wt, &st("open", "pass"));
        let got = frames(&mut rx);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0]["event"], "pr.changed");
        assert_eq!(got[0]["worktree"], "C:/axl-prchanged/wt-a");
        assert_eq!((got[0]["number"].as_u64(), got[0]["state"].as_str(), got[0]["ci"].as_str()), (Some(3), Some("open"), Some("pass")));
    }

    /// A PR that is no longer open also wakes the janitor (its branch may be removable now); an open one does not.
    #[test]
    fn a_pr_that_closed_wakes_the_janitor() {
        let (reg, mut rx) = registry();
        let before = reg.janitor.now();
        let open = Change { worktree: "w".into(), number: 1, state: "open".into(), ci: "pass".into() };
        emit(&reg, &[open]);
        assert_eq!(reg.janitor.now(), before);
        let merged = Change { worktree: "w".into(), number: 1, state: "merged".into(), ci: "pass".into() };
        emit(&reg, &[merged]);
        assert_eq!(reg.janitor.now(), before + 1);
        assert_eq!(frames(&mut rx).len(), 2);
    }

    /// A pass re-reads (through gh) only the worktrees whose look is stale: with the table full of young looks, an
    /// event pass touches nothing and says nothing.
    #[test]
    fn a_pass_over_young_looks_asks_nobody_and_says_nothing() {
        let (reg, mut rx) = registry();
        let wt = std::path::PathBuf::from("C:/axl-prchanged/wt-young");
        seen_by_app(&reg, &wt, &st("open", "pass"));
        let mut refusals = std::collections::HashSet::new();
        pass(&reg, &mut refusals, true);
        pass(&reg, &mut refusals, false);
        // (the table is shared with the other tests: only this worktree is ours to judge)
        assert!(frames(&mut rx).iter().all(|f| f["worktree"] != "C:/axl-prchanged/wt-young"));
    }
}
