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
            // a failure before the run thread exists gives the claim back
            let release = |e: anyhow::Error| {
                reg.runs.lock_recover().insert(pr.id.clone(), run_error(&name, &e));
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
                let opts = axelstra_core::orch::Options {
                    wt_root: repo.parent().unwrap_or(&repo).join(".axelstra-wt"),
                    repo,
                    name: n2.clone(),
                    max_concurrency: max,
                    agent_override: agent,
                    poll_ms: 500,
                    runner: Some(Arc::new(runner)),
                };
                // the UI sets this only after the user confirmed the acceptance commands
                let res = match &pin {
                    Some(pin) => axelstra_core::orch::run_pinned(&opts, pin),
                    None if trust => axelstra_core::orch::run_trusted(&opts),
                    None => axelstra_core::orch::run(&opts),
                };
                let v = match res {
                    Ok(r) => json!({"state": "done", "name": n2, "integ_branch": r.integ_branch,
                        "integ_wt": r.integ_wt, "merged": r.merged, "failed": r.failed,
                        "needs_review": r.needs_review, "blocked": r.blocked,
                        "reason_codes": r.reason_codes.iter().map(|(id, c)| (id.clone(), c.fields())).collect::<serde_json::Map<_, _>>()}),
                    Err(e) => run_error(&n2, &e),
                };
                reg2.runs.lock_recover().insert(pid, v);
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
            Ok(json!({}))
        }
        "orch.status" => {
            let id = p["project"].as_str().unwrap_or_default();
            Ok(reg.runs.lock_recover().get(id).cloned().unwrap_or(json!({"state": "idle"})))
        }
        "kanban.list" | "kanban.save" | "kanban.move" | "kanban.delete" => {
            let pr = crate::projects::find(&reg.store, p["project"].as_str().unwrap_or_default())?;
            let repo = std::path::PathBuf::from(&pr.path);
            let s = |k: &str| p[k].as_str().unwrap_or_default().to_string();
            tokio::task::block_in_place(|| -> Result<Value> {
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
            })
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
            Ok(serde_json::to_value(a)?)
        }
        "automation.delete" => {
            let _g = reg.auto_lock.lock_recover();
            let path = crate::automations::store_path(&reg.store);
            let mut list = crate::automations::load(&path);
            list.retain(|x| x.id != p["id"].as_str().unwrap_or_default());
            crate::automations::save_all(&path, &list)?;
            Ok(json!({}))
        }
        "automation.run" => {
            let path = crate::automations::store_path(&reg.store);
            tokio::task::block_in_place(|| run_automation(reg, &path, p["id"].as_str().unwrap_or_default()))
        }
        m => anyhow::bail!("unknown method {m}"),
    }
}
