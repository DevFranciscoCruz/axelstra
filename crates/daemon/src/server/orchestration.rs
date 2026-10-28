use super::*;

/// The `error` state of a run: English text plus, when the error has a message code, `error_code`/`error_params` for the UI.
fn run_error(name: &str, e: &anyhow::Error) -> Value {
    let mut v = json!({"state": "error", "name": name, "error": format!("{e:#}")});
    if let Some(c) = axelstra_core::msg::find(e) {
        v["error_code"] = json!(c.code);
        v["error_params"] = c.params_json();
    }
    v
}

impl Registry {
    /// A card or the board of `project` changed: the app reloads it instead of asking on a timer.
    pub(super) fn kanban_changed(&self, project: &str) {
        self.broadcast_event(json!({"event": "kanban.changed", "project": project}));
    }

    /// The state of the run `run` (its name, `orch/<run>`) of `project` changed: started, a card moved, finished, retried.
    pub(super) fn orch_changed(&self, run: &str, project: &str) {
        self.broadcast_event(json!({"event": "orch.changed", "run": run, "project": project}));
    }
}

/// Watched projects, per registry (several daemons can live in one test process).
static KANBAN_WATCHES: Mutex<Vec<(usize, String)>> = Mutex::new(Vec::new());

/// Makes sure the `.kanban` folder of the project is watched: a change anywhere (the orchestrator moving a card, an
/// agent or an editor writing one) becomes `kanban.changed`, and `orch.changed` while a run is going. A project with no
/// folder yet is watched from the first call after the folder exists.
fn ensure_kanban_watch(reg: &Arc<Registry>, project: &str, repo: &std::path::Path) {
    let key = (Arc::as_ptr(reg) as usize, project.to_string());
    {
        let mut w = KANBAN_WATCHES.lock_recover();
        if w.contains(&key) || !repo.join(".kanban").is_dir() {
            return;
        }
        w.push(key.clone());
    }
    let (weak, pid) = (Arc::downgrade(reg), project.to_string());
    let started = crate::kanban::watch(
        repo,
        move || {
            let Some(reg) = weak.upgrade() else { return false };
            reg.kanban_changed(&pid);
            let run =
                reg.runs.lock_recover().get(&pid).filter(|r| r["state"] == "running").and_then(|r| r["name"].as_str().map(String::from));
            if let Some(run) = run {
                reg.orch_changed(&run, &pid);
            }
            true
        },
        {
            let key = key.clone();
            move || KANBAN_WATCHES.lock_recover().retain(|k| k != &key)
        },
    );
    if started.is_err() {
        KANBAN_WATCHES.lock_recover().retain(|k| k != &key);
    }
}

pub(super) fn call(c: &mut Ctx) -> Result<Value> {
    let req = c.req;
    let (p, reg) = (c.p, c.reg);
    match req.method.as_str() {
        "orch.limits" => {
            if let Some(n) = p["max_workers"].as_u64() {
                reg.bus.set_limit(n as usize);
            }
            let alive = |i: u32| reg.get(i).map(|s| s.try_wait().is_none()).unwrap_or(false);
            Ok(json!({"max_workers": reg.bus.limit(), "workers": reg.bus.live_workers(alive)}))
        }
        "orch.trust_info" => {
            let pr = crate::projects::find(&reg.store, p["project"].as_str().unwrap_or_default())?;
            let repo = std::path::PathBuf::from(&pr.path);
            Ok(serde_json::to_value(tokio::task::block_in_place(|| axelstra_core::orch::trust_info(&repo))?)?)
        }
        "orch.start" => {
            let pr = crate::projects::find(&reg.store, p["project"].as_str().unwrap_or_default())?;
            let name = format!("ui-{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs());
            let started = json!({"state": "running", "name": name, "integ_branch": format!("orch/{name}")});
            // check and claim in one step: two calls arriving together cannot both find the project idle
            {
                let mut runs = reg.runs.lock_recover();
                if runs.get(&pr.id).is_some_and(|r| r["state"] == "running") {
                    return Err(axelstra_core::coded!("orch.already_running", "an orchestration is already running for this project"));
                }
                runs.insert(pr.id.clone(), started.clone());
            }
            let cancel = axelstra_core::orch::Cancel::new();
            reg.run_cancels.lock_recover().insert(pr.id.clone(), cancel.clone());
            reg.orch_changed(&name, &pr.id);
            ensure_kanban_watch(reg, &pr.id, std::path::Path::new(&pr.path));
            // a failure before the run thread exists gives the claim back
            let release = |e: anyhow::Error| {
                reg.run_cancels.lock_recover().remove(&pr.id);
                reg.runs.lock_recover().insert(pr.id.clone(), run_error(&name, &e));
                reg.orch_changed(&name, &pr.id);
                e
            };
            let repo = axelstra_core::git::canon(std::path::Path::new(&pr.path)).map_err(release)?;
            let max = p["max"].as_u64().unwrap_or(3).clamp(1, 8) as usize;
            let trust = p["trust_acceptance"].as_bool().unwrap_or(false);
            // `bypass*` cards run only against the fingerprint of the cards the user confirmed (orch::trust_info)
            let pin: Option<String> = p["pin"].as_str().map(String::from);
            // overriding the agent command is for tests and custom agents; the same
            // client can already spawn any process, so this grants nothing new
            let agent: Option<Vec<String>> = p["agent"].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect());
            // connects back to this same daemon: must not hold an async worker
            let runner = tokio::task::block_in_place(|| crate::runner::DaemonRunner::connect(&reg.pipe, &reg.token)).map_err(release)?;
            let (reg2, pid, n2) = (reg.clone(), pr.id.clone(), name.clone());
            std::thread::spawn(move || {
                let wt_root = repo.parent().unwrap_or(&repo).join(".axelstra-wt");
                // the run's worktrees are Axelstra's own: the janitor may clean them up once they are integrated or empty
                crate::orchend::mark_run(&reg2.store, &pid, &repo, &wt_root, &n2);
                // the folder the run's worktrees go into is kept out of the indexers before the first one exists
                super::scanx::run_root_ready(&reg2, &repo, &wt_root);
                let opts = axelstra_core::orch::Options {
                    wt_root,
                    repo,
                    name: n2.clone(),
                    max_concurrency: max,
                    agent_override: agent,
                    poll_ms: 500,
                    cancel: Some(cancel),
                    runner: Some(Arc::new(runner)),
                };
                // the UI sets this only after the user confirmed the acceptance commands
                let res = match &pin {
                    Some(pin) => axelstra_core::orch::run_pinned(&opts, pin),
                    None if trust => axelstra_core::orch::run_trusted(&opts),
                    None => axelstra_core::orch::run(&opts),
                };
                // still "running" for the janitor while this cleans up, so it cannot race the run's own worktrees
                if let Ok(pr) = crate::projects::find(&reg2.store, &pid) {
                    let end = crate::orchend::end_of(&res);
                    crate::orchend::finish(&pr, &n2, &end, &|path| reg2.close_worktree(path));
                }
                let v = match res {
                    Ok(r) => json!({"state": "done", "name": n2, "integ_branch": r.integ_branch,
                        "integ_wt": r.integ_wt, "merged": r.merged, "failed": r.failed,
                        "needs_review": r.needs_review, "blocked": r.blocked, "cancelled": r.cancelled,
                        "reason_codes": r.reason_codes.iter().map(|(id, c)| (id.clone(), c.fields())).collect::<serde_json::Map<_, _>>()}),
                    Err(e) => run_error(&n2, &e),
                };
                reg2.run_cancels.lock_recover().remove(&pid);
                reg2.runs.lock_recover().insert(pid.clone(), v);
                reg2.orch_changed(&n2, &pid);
                reg2.kanban_changed(&pid);
                // the run removes worktrees with git directly: their sessions go, the app hears of it, the janitor looks
                reap_orphans(&reg2);
                reg2.projects_changed();
                reg2.announce_events();
                reg2.janitor.bump();
            });
            Ok(started)
        }
        "orch.retry" => {
            let pr = crate::projects::find(&reg.store, p["project"].as_str().unwrap_or_default())?;
            let id = p["id"].as_str().unwrap_or_default().to_string();
            let name = reg.runs.lock_recover().get(&pr.id).and_then(|r| r["name"].as_str().map(String::from));
            let name = name.ok_or_else(|| axelstra_core::coded!("orch.no_run", "no orchestration run to retry"))?;
            let trust = p["trust_acceptance"].as_bool().unwrap_or(false);
            let repo = axelstra_core::git::canon(std::path::Path::new(&pr.path))?;
            let wt_root = repo.parent().unwrap_or(&repo).join(".axelstra-wt");
            tokio::task::block_in_place(|| axelstra_core::orch::retry_merge(&repo, &wt_root, &name, &id, trust))?;
            if let Some(r) = reg.runs.lock_recover().get_mut(&pr.id) {
                if let Some(v) = r["needs_review"].as_array_mut() {
                    v.retain(|e| e[0] != id.as_str());
                }
            }
            reg.orch_changed(&name, &pr.id);
            reg.kanban_changed(&pr.id);
            // the last card waiting for review is merged: the run is finished after all, so it is cleaned up and integrated
            let state = reg.runs.lock_recover().get(&pr.id).cloned().unwrap_or_default();
            if crate::orchend::state_is_clean(&state) {
                let end = crate::orchend::End { clean: true, holds: false };
                tokio::task::block_in_place(|| crate::orchend::finish(&pr, &name, &end, &|path| reg.close_worktree(path)));
                reap_orphans(reg);
                reg.projects_changed();
                reg.announce_events();
                reg.orch_changed(&name, &pr.id);
            }
            Ok(json!({}))
        }
        // stops the run of the project: its thread wakes at once, kills the workers' sessions, puts the unfinished cards back
        // to todo and ends through the usual cleanup (`orchend::finish`: work already merged is kept, never integrated)
        "orch.cancel" => {
            // by project id, or by the name of the run (`orch.status` `name`) when only that is at hand
            let by_run = p["run"].as_str().and_then(|run| {
                reg.runs
                    .lock_recover()
                    .iter()
                    .find(|(_, r)| r["name"].as_str() == Some(run) && r["state"] == "running")
                    .map(|(k, _)| k.clone())
            });
            let pr = crate::projects::find(&reg.store, p["project"].as_str().map(String::from).or(by_run).unwrap_or_default().as_str())?;
            let cancel = reg.run_cancels.lock_recover().get(&pr.id).cloned();
            let (Some(cancel), Some(name)) = (
                cancel,
                reg.runs.lock_recover().get(&pr.id).filter(|r| r["state"] == "running").and_then(|r| r["name"].as_str().map(String::from)),
            ) else {
                return Err(axelstra_core::coded!("orch.not_running", "no orchestration is running for this project"));
            };
            cancel.cancel();
            if let Some(r) = reg.runs.lock_recover().get_mut(&pr.id) {
                r["cancelling"] = json!(true);
            }
            reg.orch_changed(&name, &pr.id);
            Ok(json!({"run": name}))
        }
        "orch.status" => {
            let id = p["project"].as_str().unwrap_or_default();
            Ok(reg.runs.lock_recover().get(id).cloned().unwrap_or(json!({"state": "idle"})))
        }
        "kanban.list" | "kanban.save" | "kanban.move" | "kanban.delete" => {
            let pr = crate::projects::find(&reg.store, p["project"].as_str().unwrap_or_default())?;
            let repo = std::path::PathBuf::from(&pr.path);
            let s = |k: &str| p[k].as_str().unwrap_or_default().to_string();
            let out = tokio::task::block_in_place(|| -> Result<Value> {
                use crate::kanban;
                Ok(match req.method.as_str() {
                    "kanban.list" => serde_json::to_value(kanban::list(&repo)?)?,
                    "kanban.save" => {
                        serde_json::to_value(kanban::save(&repo, p["id"].as_str(), &s("title"), &s("body"), p["status"].as_str(), p)?)?
                    }
                    "kanban.move" => {
                        kanban::set_status(&repo, &s("id"), &s("status"))?;
                        json!({})
                    }
                    _ => {
                        kanban::delete(&repo, &s("id"))?;
                        json!({})
                    }
                })
            });
            ensure_kanban_watch(reg, &pr.id, &repo);
            if out.is_ok() && req.method != "kanban.list" {
                reg.kanban_changed(&pr.id);
            }
            out
        }
        "automation.list" => {
            let _g = reg.auto_lock.lock_recover();
            Ok(json!({"automations": crate::automations::load(&crate::automations::store_path(&reg.store))}))
        }
        "automation.save" => {
            let _g = reg.auto_lock.lock_recover();
            let path = crate::automations::store_path(&reg.store);
            let mut a: crate::automations::Automation = serde_json::from_value(p.clone())
                .map_err(|e| axelstra_core::coded!("automation.invalid", "invalid automation: {detail}", detail = e))?;
            crate::automations::validate(&a)?;
            crate::projects::find(&reg.store, &a.project)?;
            let mut list = crate::automations::load(&path);
            match list.iter_mut().find(|x| !a.id.is_empty() && x.id == a.id) {
                Some(old) => {
                    // editing keeps the run history; a changed schedule starts counting from now
                    if serde_json::to_value(&old.schedule)? != serde_json::to_value(&a.schedule)? {
                        a.last_run = crate::automations::now_ts();
                    } else {
                        a.last_run = old.last_run;
                    }
                    *old = a.clone();
                }
                None => {
                    a.id = format!("auto-{}", crate::automations::now_ts());
                    while list.iter().any(|x| x.id == a.id) {
                        a.id.push('x');
                    }
                    a.last_run = crate::automations::now_ts();
                    list.push(a.clone());
                }
            }
            crate::automations::save_all(&path, &list)?;
            // the scheduler sleeps until the next due run: it works that out again with this one
            reg.auto_wake.notify_one();
            reg.broadcast("automations.changed");
            Ok(serde_json::to_value(a)?)
        }
        "automation.delete" => {
            let _g = reg.auto_lock.lock_recover();
            let path = crate::automations::store_path(&reg.store);
            let mut list = crate::automations::load(&path);
            list.retain(|x| x.id != p["id"].as_str().unwrap_or_default());
            crate::automations::save_all(&path, &list)?;
            reg.auto_wake.notify_one();
            reg.broadcast("automations.changed");
            Ok(json!({}))
        }
        "automation.run" => {
            let path = crate::automations::store_path(&reg.store);
            let res = tokio::task::block_in_place(|| run_automation(reg, &path, p["id"].as_str().unwrap_or_default()));
            // its last run moved: so does its next one
            reg.auto_wake.notify_one();
            // its last run moved even when the start failed
            reg.broadcast("automations.changed");
            res
        }
        m => Err(unknown_method(m)),
    }
}
