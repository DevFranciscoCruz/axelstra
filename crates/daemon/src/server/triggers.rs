//! `triggers.*`: the CI-fix trigger of a project (see `crate::triggers`) and the loop that runs it.
use super::*;
use crate::triggers as trig;
use axelstra_core::paths;

/// `Env` of the running daemon: real `gh`, real sessions.
struct DaemonEnv(Arc<Registry>);

impl trig::Env for DaemonEnv {
    fn now(&self) -> u64 {
        u64::try_from(crate::automations::now_ts()).unwrap_or(0)
    }

    fn projects(&self) -> Vec<crate::projects::Project> {
        crate::projects::load(&self.0.store)
    }

    fn worktrees(&self, p: &crate::projects::Project) -> Vec<axelstra_core::git::Worktree> {
        axelstra_core::git::worktree_list(std::path::Path::new(&p.path)).unwrap_or_default()
    }

    fn live(&self, project: &str, worktree: &str) -> trig::Live {
        let want = paths::norm(worktree);
        let sessions = self.0.sessions.lock_recover();
        let metas = self.0.metas.lock_recover();
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
        crate::checks::status(worktree)
    }

    fn spawn_fix(&self, p: &crate::projects::Project, worktree: &str, prompt: &str) -> Result<u32> {
        // AXELSTRA_AGENT_CLI swaps the agent binary (a wrapper script, or a harmless stand-in in tests)
        let (program, args) = match std::env::var("AXELSTRA_AGENT_CLI") {
            Ok(cli) => crate::automations::raw_command(&cli, prompt),
            Err(_) => {
                let ag = axelstra_core::agents::require(trig::AGENT)?;
                if ag.locate().is_none() {
                    anyhow::bail!("{}", ag.missing_message());
                }
                crate::automations::agent_command(trig::AGENT, prompt)?
            }
        };
        // a task of the worktree like any other: `session.spawn` names it and makes the lead its parent. The agent
        // runs with its default permissions: nothing here asks for a bypass mode.
        let meta = json!({"project": p.id, "worktree": worktree, "kind": "agent", "agent": trig::AGENT, "trigger": "ci_fix"});
        let mut client = crate::client::Client::connect(&self.0.pipe, &self.0.token)?;
        let r = client.call(
            "session.spawn",
            json!({"program": program, "args": args, "cwd": worktree, "label": trig::AGENT, "meta": meta,
                   "goal": "Corrigir o CI (disparado automaticamente)", "cols": 120, "rows": 30}),
        )?;
        r["id"].as_u64().and_then(|i| u32::try_from(i).ok()).ok_or_else(|| anyhow::anyhow!("daemon returned no id"))
    }
}

/// Looks at the CI of the opted-in projects every `POLL_SECS`; a project that did not opt in costs one small file read.
pub(super) fn spawn_loop(reg: Arc<Registry>) {
    std::thread::spawn(move || {
        let mut seen = std::collections::HashSet::new();
        std::thread::sleep(Duration::from_secs(30));
        loop {
            let path = trig::store_path(&reg.store);
            let fired = trig::tick(&DaemonEnv(reg.clone()), &path, &reg.audit, &mut seen);
            for o in &fired {
                eprintln!("triggers: {} {} {}: {}", o.kind, o.project, o.worktree, o.detail);
            }
            if !fired.is_empty() {
                // sessions changed: watchers refresh
                reg.wt_gen.fetch_add(1, Ordering::SeqCst);
            }
            std::thread::sleep(Duration::from_secs(trig::POLL_SECS));
        }
    });
}

fn limits(t: &trig::ProjectTriggers) -> Value {
    json!({"cooldown_secs": t.cooldown_secs, "max_per_hour": t.max_per_hour, "poll_secs": trig::POLL_SECS,
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
                        None => anyhow::bail!("{k} must be a whole number from {lo} to {hi}"),
                    },
                }
            };
            let cooldown = int("cooldown_secs", trig::COOLDOWN_RANGE.0, trig::COOLDOWN_RANGE.1)?;
            let max = int("max_per_hour", u64::from(trig::MAX_PER_HOUR_RANGE.0), u64::from(trig::MAX_PER_HOUR_RANGE.1))?;
            let want = match p.get("ci_fix") {
                None | Some(Value::Null) => None,
                Some(v) => Some(v.as_bool().ok_or_else(|| anyhow::anyhow!("ci_fix must be true or false"))?),
            };
            // the first enabling is something the user typed, not a flag
            if want == Some(true) && t.confirmed_at.is_none() {
                if p["confirm"].as_str() != Some(trig::CONFIRM_WORD) {
                    anyhow::bail!("enabling needs confirm = \"{}\" the first time", trig::CONFIRM_WORD);
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
        m => anyhow::bail!("unknown method {m}"),
    }
}
