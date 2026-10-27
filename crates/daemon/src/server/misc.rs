use super::*;

pub(super) fn call(c: &mut Ctx) -> Result<Value> {
    let req = c.req;
    let (p, reg) = (c.p, c.reg);
    let (authed, cli, agent, peer) = (&mut *c.authed, &mut *c.cli, &mut *c.agent, c.peer);
    let token = c.token;
    match req.method.as_str() {
        "hello" => {
            // the session a presented capability token belongs to; compared against every issued token, not looked up,
            // so the time taken does not tell how close a guess is
            let agent_session = p.get("agent_token").and_then(Value::as_str).and_then(|t| {
                reg.agent_tokens.lock_recover().iter().fold(None, |found, (k, id)| if ct_eq(k, t) { Some(*id) } else { found })
            });
            let token_ok = p.get("token").and_then(Value::as_str).is_some_and(|t| ct_eq(t, token));
            if agent_session.is_none() && !token_ok {
                reg.hello_failed();
                return Err(axelstra_core::coded!("rpc.bad_token", "bad token"));
            }
            reg.bad_hellos.store(0, Ordering::SeqCst);
            *authed = true;
            // the token proves the caller may use the daemon; what the caller is comes from the process on the other end.
            // A process inside an agent's tree is that agent however it said hello: the master token it read from
            // the data dir (or its env) buys it nothing. The credential only names the session for a caller the OS
            // cannot examine, and never overrides what the OS says.
            *agent = reg.origin_of(peer).or(agent_session);
            *cli = agent.is_some() || p.get("client").and_then(Value::as_str) == Some("cli");
            Ok(hello_info())
        }
        "agent.status" => {
            let key = p["key"].as_str().unwrap_or_default();
            if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
                return Err(axelstra_core::coded!("agent.bad_status_key", "bad key"));
            }
            let file = reg.saved.parent().unwrap_or(std::path::Path::new("")).join("status").join(format!("{key}.json"));
            let status = std::fs::read_to_string(&file).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok());
            Ok(json!({"status": status}))
        }
        "flow.set" => {
            let id = p["flow"].as_str().unwrap_or_default().to_string();
            if !id.is_empty() {
                reg.flows.lock_recover().insert(id, p["tasks"].clone());
            }
            Ok(json!({}))
        }
        "flow.clear" => {
            reg.flows.lock_recover().remove(p["flow"].as_str().unwrap_or_default());
            // its worktrees are no longer held by the plan
            reg.janitor.bump();
            Ok(json!({}))
        }
        "flow.list" => {
            let all: Vec<Value> = reg.flows.lock_recover().values().flat_map(|t| t.as_array().cloned().unwrap_or_default()).collect();
            Ok(json!({"tasks": all}))
        }
        "agents.list" => Ok(json!({"agents": tokio::task::block_in_place(axelstra_core::agents::list)})),
        "codex.limits" => Ok(json!({"limits": tokio::task::block_in_place(crate::agentlog::codex_limits)})),
        // the gate lives here, not in the CLI: while screen control is off nothing below runs, whoever asks
        m if m.starts_with("computer.") && !matches!(m, "computer.status" | "computer.enable") => {
            if !reg.computer.load(Ordering::SeqCst) {
                return Err(axelstra_core::coded!("computer.off", "screen control is off. The user turns it on in the Axelstra app (command palette: «Screen control»); an agent cannot enable it"));
            }
            tokio::task::block_in_place(|| crate::computer::call(m, p))
        }
        "computer.status" => Ok(json!({"enabled": reg.computer.load(Ordering::SeqCst)})),
        "computer.enable" => {
            // agents run the CLI: only the app (a connection that did not say it is the CLI) may turn it on
            if *cli && p["on"].as_bool() == Some(true) {
                return Err(axelstra_core::coded!("computer.user_only", "screen control can only be turned on by the user in the app"));
            }
            reg.computer.store(p["on"].as_bool() == Some(true), Ordering::SeqCst);
            Ok(json!({"enabled": reg.computer.load(Ordering::SeqCst)}))
        }
        "app.dir" => Ok(json!({"dir": reg.saved.parent().unwrap_or(std::path::Path::new("")).to_string_lossy().replace('\\', "/")})),
        "settings.get" => {
            let saved = reg.settings();
            // `terse` in the request asks for the rules of that level instead of the saved one (a project's own choice)
            let terse = p["terse"].as_str().or(saved["terse"].as_str()).filter(|l| ["off", "short", "tiny"].contains(l)).unwrap_or("off");
            let installed = tokio::task::block_in_place(|| crate::projects::package_managers(None, None).0);
            let pm = saved["pm"].as_str().filter(|m| installed.contains(m)).map(String::from);
            let (shell, notify_sound, notify_mode) = ui_settings(&saved);
            Ok(
                json!({"terse": terse, "rules": crate::skill::terse_rules(terse), "pm": pm, "installed": installed, "auto_install": saved["auto_install"] == json!(true),
                "shell": shell, "notify_sound": notify_sound, "notify_mode": notify_mode, "stall_minutes": crate::stall::minutes(&saved), "ui": saved["ui"].as_object().cloned().unwrap_or_default(),
                "budgets": crate::budget::validate(&saved["budgets"]).unwrap_or_else(|_| json!({})),
                "hosts": crate::hosts::validate(&saved["hosts"]).unwrap_or_else(|_| json!([]))}),
            )
        }
        "settings.set" => {
            // partial update: only the keys present are changed
            let mut saved = reg.settings();
            if let Some(level) = p["terse"].as_str() {
                if !["off", "short", "tiny"].contains(&level) {
                    return Err(axelstra_core::coded!("settings.bad_terse", "terse must be off, short or tiny"));
                }
                saved["terse"] = json!(level);
            }
            if p.get("pm").is_some() {
                saved["pm"] = match p["pm"].as_str() {
                    Some(m) if ["bun", "pnpm", "yarn", "npm"].contains(&m) => json!(m),
                    Some(_) => return Err(axelstra_core::coded!("settings.bad_pm", "pm must be bun, pnpm, yarn or npm")),
                    None => Value::Null,
                };
            }
            if let Some(b) = p["auto_install"].as_bool() {
                saved["auto_install"] = json!(b);
            }
            apply_ui_settings(&mut saved, p)?;
            if let Some(v) = p.get("stall_minutes") {
                let Some(m) = crate::stall::validate(v) else {
                    return Err(axelstra_core::coded!(
                        "settings.bad_stall_minutes",
                        "stall_minutes must be a whole number from 0 (off) to 1440"
                    ));
                };
                saved["stall_minutes"] = json!(m);
            }
            // replaces the whole budgets setting; null removes it
            if let Some(b) = p.get("budgets") {
                saved["budgets"] = if b.is_null() { json!({}) } else { crate::budget::validate(b)? };
            }
            // replaces the whole host registry; null removes it
            if let Some(h) = p.get("hosts") {
                saved["hosts"] = crate::hosts::validate(h)?;
            }
            let written = write_json(&reg.saved.with_file_name("settings.json"), &saved);
            reg.settings_changed();
            written?;
            // the budget loop sleeps while there is no budget: it reads the settings again
            reg.budget_wake.notify_one();
            Ok(json!({}))
        }
        "agent.search" => {
            let q = p["query"].as_str().unwrap_or_default().to_string();
            let max = p["max"].as_u64().unwrap_or(30).clamp(1, 200) as usize;
            let Some(me) = *agent else {
                let hits = tokio::task::block_in_place(|| crate::agentlog::search(&q, max));
                return Ok(json!({"hits": hits}));
            };
            // an agent reads only the conversations of its own project: its checkout and worktrees. A hit with no folder
            // (Gemini keeps only a hash) cannot be placed, so it stays out.
            let project = reg.metas.lock_recover().get(&me).and_then(|m| m["project"].as_str().map(String::from));
            let roots: Vec<String> = project
                .and_then(|id| crate::projects::find(&reg.store, &id).ok())
                .map(|pr| {
                    let mut v = vec![pr.path.clone()];
                    v.extend(
                        axelstra_core::git::worktree_list(std::path::Path::new(&pr.path)).unwrap_or_default().into_iter().map(|w| w.path),
                    );
                    v
                })
                .unwrap_or_default();
            let mut hits = tokio::task::block_in_place(|| crate::agentlog::search(&q, 200));
            hits.retain(|h| !h.cwd.is_empty() && roots.iter().any(|r| axelstra_core::paths::is_within(&h.cwd, r)));
            hits.truncate(max);
            Ok(json!({"hits": hits}))
        }
        "agent.usage" => {
            let cwd = p["cwd"].as_str().unwrap_or_default().to_string();
            let u = tokio::task::block_in_place(|| crate::agentlog::claude_usage(&cwd, p["session"].as_str()));
            Ok(json!({"usage": u}))
        }
        "agent.worktree_usage" => {
            let paths: Vec<String> =
                p["worktrees"].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect()).unwrap_or_default();
            let usage = tokio::task::block_in_place(|| crate::agentlog::worktree_usage(&paths));
            Ok(json!({"usage": usage}))
        }
        // spending against the token limits (read only); `worktrees` limits it to those paths, else every project worktree
        "budget.status" => {
            let budgets = reg.with_settings(crate::budget::parse);
            let asked: Option<Vec<String>> =
                p["worktrees"].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect());
            let rows = tokio::task::block_in_place(|| {
                let paths = asked.unwrap_or_else(|| budget_paths(reg, &budgets));
                let totals = budget_totals(&paths);
                let state = read_json::<crate::budget::State>(&budget_state_path(reg)).ok().flatten().unwrap_or_default();
                crate::budget::status(&state, &budgets, &totals, chrono::Local::now().date_naive())
            });
            Ok(json!({"budgets": rows}))
        }
        // evaluates the limits now (the scheduler does it every two minutes), e.g. right after the user changed them
        "budget.check" => Ok(json!({"budgets": tokio::task::block_in_place(|| budget_check(reg))})),
        // the level crossings since the last call (the app turns them into a toast and a notification)
        "budget.events" => Ok(json!({"events": crate::budget::take_events()})),
        // what this machine is and which folders it has, for the app's "add remote project" picker (app only)
        "host.info" => Ok(crate::hosts::info()),
        "host.browse" => tokio::task::block_in_place(|| crate::hosts::browse(p["path"].as_str())),
        // `agent` (claude by default, or gemini) picks whose settings file is read or written
        "hooks.status" => Ok(crate::hooks::status_for(p["agent"].as_str().unwrap_or("claude"))?),
        "hooks.install" => tokio::task::block_in_place(|| crate::hooks::install_for(p["agent"].as_str().unwrap_or("claude"))),
        "ping" => Ok(hello_info()),
        // what the daemon recovered or lost on its own (a state file moved aside, restored from a backup): shown once
        "audit.list" => {
            let limit = p["limit"].as_u64().map_or(100, |n| n.clamp(1, 500) as usize);
            let session = p["session"].as_u64().and_then(|x| u32::try_from(x).ok());
            Ok(json!({"entries": reg.audit.list(limit, session)}))
        }
        "env.tools" => Ok(json!({"tools": tokio::task::block_in_place(crate::envtools::detect)})),
        "backup.export" => {
            // sessions.json is the registry's own view, not whatever was last flushed
            reg.persist();
            let dir = reg.saved.parent().unwrap_or(std::path::Path::new("")).to_path_buf();
            let out = tokio::task::block_in_place(|| crate::backup::export(&dir))?;
            Ok(json!({"path": out.to_string_lossy().replace('\\', "/")}))
        }
        "backup.restore" => {
            let src = p["path"]
                .as_str()
                .filter(|s| !s.is_empty())
                .ok_or_else(|| axelstra_core::coded!("workspace.path_required", "path is required"))?;
            let dir = reg.saved.parent().unwrap_or(std::path::Path::new("")).to_path_buf();
            let restored = tokio::task::block_in_place(|| crate::backup::restore(&dir, std::path::Path::new(src)));
            reg.settings_changed();
            let restored = restored?;
            if restored.iter().any(|f| f == "sessions.json") {
                // the restored sessions wait for the UI like those of a daemon restart; live ones are kept
                let v = read_json::<Vec<Value>>(&reg.saved).ok().flatten().unwrap_or_default();
                *reg.pending.lock_recover() = v;
                reg.persist();
            }
            *reg.plist.lock_recover() = None;
            reg.wt_gen.fetch_add(1, Ordering::SeqCst);
            Ok(json!({"restored": restored}))
        }
        // one JSON file with projects, automations, settings and kanban cards; never tokens or sessions. The paths come
        // from the user (native dialog or CLI), which is why agents are refused all three
        "workspace.export" => {
            let path = p["path"]
                .as_str()
                .filter(|s| !s.is_empty())
                .ok_or_else(|| axelstra_core::coded!("workspace.path_required", "path is required"))?;
            let sum = tokio::task::block_in_place(|| crate::workspace::export_to(&reg.store, std::path::Path::new(path)))?;
            Ok(json!({"path": path.replace('\\', "/"), "summary": sum}))
        }
        "workspace.import_preview" => {
            let path = p["path"].as_str().unwrap_or_default();
            let map = crate::workspace::parse_map(&p["map"])?;
            let pv = tokio::task::block_in_place(|| crate::workspace::preview(&reg.store, path, &map))?;
            Ok(serde_json::to_value(pv)?)
        }
        "workspace.import" => {
            let path = p["path"].as_str().unwrap_or_default();
            let replace = match p["mode"].as_str().unwrap_or("merge") {
                "merge" => false,
                "replace" => true,
                m => return Err(axelstra_core::coded!("workspace.bad_mode", "mode must be merge or replace, not {m:?}", m = m)),
            };
            let overwrite = match p["conflicts"].as_str().unwrap_or("keep") {
                "keep" => false,
                "overwrite" => true,
                m => return Err(axelstra_core::coded!("workspace.bad_conflicts", "conflicts must be keep or overwrite, not {m:?}", m = m)),
            };
            let o = crate::workspace::Options { replace, overwrite, map: crate::workspace::parse_map(&p["map"])? };
            let out = {
                let _g = reg.auto_lock.lock_recover();
                let out = tokio::task::block_in_place(|| crate::workspace::import(&reg.store, path, &o));
                // it may have replaced settings.json, even when it failed midway
                reg.settings_changed();
                out?
            };
            *reg.plist.lock_recover() = None;
            reg.wt_gen.fetch_add(1, Ordering::SeqCst);
            Ok(serde_json::to_value(out)?)
        }
        "notices.take" => {
            // `notices` stays the English lines; `items` (additive) pairs each with its message code, when it has one
            let items = crate::store::take_notice_items();
            let items: Vec<Value> = items
                .iter()
                .map(|(t, c)| {
                    let mut o = json!({"text": t});
                    if let Some(c) = c {
                        o["code"] = json!(c.code);
                        o["params"] = c.params_json();
                    }
                    o
                })
                .collect();
            let notices: Vec<&str> = items.iter().filter_map(|i| i["text"].as_str()).collect();
            Ok(json!({"notices": notices, "items": items}))
        }
        // used by the app updater: the installer cannot replace a running axelstra.exe.
        // Sessions die with the daemon (Job Objects), so the UI warns before calling this.
        "daemon.shutdown" => {
            // remember what was going on, so the next start can pick it up: agents that were working, and the
            // ephemeral workers that die with this daemon (they are not restored) per coordinator
            let live: Vec<(u32, bool)> = reg
                .sessions
                .lock_recover()
                .iter()
                .filter(|(_, s)| s.try_wait().is_none())
                .map(|(id, s)| (*id, s.idle_ms() < 5000))
                .collect();
            let mut working: Vec<u32> = vec![];
            let mut lost: HashMap<u32, Vec<String>> = HashMap::new();
            {
                let metas = reg.metas.lock_recover();
                for (id, busy) in &live {
                    let m = metas.get(id);
                    if reg.bus.is_reaped(*id) {
                        if let Some(parent) = reg.bus.parent_of(*id) {
                            let wt = m.and_then(|m| m["worktree"].as_str()).unwrap_or("?");
                            lost.entry(parent).or_default().push(wt.to_string());
                        }
                    } else if *busy && m.and_then(|m| m["kind"].as_str()) == Some("agent") {
                        working.push(*id);
                    }
                }
            }
            {
                let mut specs = reg.specs.lock_recover();
                for (id, spec) in specs.iter_mut() {
                    if working.contains(id) {
                        spec["meta"]["interrupted"] = Value::Bool(true);
                    }
                    if let Some(w) = lost.get(id) {
                        spec["meta"]["lost_workers"] = json!(w);
                    }
                }
            }
            reg.closing.store(true, Ordering::SeqCst);
            // a save still waiting out its debounce must not be lost with the process
            reg.persist_now();
            // keep what each agent terminal showed, so the restored tab is not empty
            save_histories(reg);
            // end every terminal and agent now (their specs stay in sessions.json, so they restore on the next start)
            for s in reg.sessions.lock_recover().values() {
                s.kill();
            }
            std::thread::spawn(|| {
                std::thread::sleep(std::time::Duration::from_millis(300));
                std::process::exit(0);
            });
            Ok(json!({}))
        }
        m => anyhow::bail!("unknown method {m}"),
    }
}

fn budget_state_path(reg: &Registry) -> std::path::PathBuf {
    reg.saved.with_file_name("budget-state.json")
}

/// Worktrees a budget applies to: every linked worktree of the projects, plus those a budget names.
fn budget_paths(reg: &Registry, budgets: &crate::budget::Budgets) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    for pr in crate::projects::load(&reg.store) {
        for w in axelstra_core::git::worktree_list(std::path::Path::new(&pr.path)).unwrap_or_default() {
            if !w.is_main {
                out.push(w.path);
            }
        }
    }
    out.extend(budgets.worktrees.iter().map(|(p, _)| p.clone()));
    let mut seen = std::collections::HashSet::new();
    out.retain(|p| seen.insert(crate::budget::norm(p)));
    out
}

/// Cumulative tokens paid for per worktree (input, output and cache writes, like the app's totals).
fn budget_totals(paths: &[String]) -> Vec<(String, u64)> {
    crate::agentlog::worktree_usage(paths).into_iter().map(|u| (u.worktree, u.input + u.output + u.cache_write)).collect()
}

/// Serializes the read-modify-write of the budget state (the scheduler and `budget.check` may overlap).
static BUDGET_LOCK: Mutex<()> = Mutex::new(());

/// Interrupts (Esc, which stops the turn but keeps the agent) the agents of a worktree that are working right now.
/// Returns the sessions it wrote to.
fn pause_agents(reg: &Registry, path: &str) -> Vec<u32> {
    let want = crate::budget::norm(path);
    let working: Vec<u32> =
        reg.sessions.lock_recover().iter().filter(|(_, s)| s.try_wait().is_none() && s.idle_ms() < 5000).map(|(id, _)| *id).collect();
    let targets: Vec<u32> = {
        let metas = reg.metas.lock_recover();
        working
            .into_iter()
            .filter(|id| {
                metas
                    .get(id)
                    .is_some_and(|m| m["kind"] == "agent" && m["worktree"].as_str().is_some_and(|w| crate::budget::norm(w) == want))
            })
            .collect()
    };
    targets.into_iter().filter(|id| reg.get(*id).is_some_and(|s| s.write(b"").is_ok())).collect()
}

/// Reads the spending, records it and reports every level crossing once per period. Only a budget the user opted
/// into (`auto_pause`) interrupts agents, and only when its limit is passed. Returns the current rows.
pub(super) fn budget_check(reg: &Arc<Registry>) -> Vec<crate::budget::Row> {
    use crate::budget;
    let budgets = reg.with_settings(budget::parse);
    if budgets.is_empty() {
        return vec![];
    }
    let totals = budget_totals(&budget_paths(reg, &budgets));
    let _guard = BUDGET_LOCK.lock_recover();
    let path = budget_state_path(reg);
    let mut state = read_json::<budget::State>(&path).ok().flatten().unwrap_or_default();
    let before = state.clone();
    let date = chrono::Local::now().date_naive();
    let alerts = budget::tick(&mut state, &budgets, &totals, date);
    for row in &alerts {
        let paused = if budget::should_pause(row) { pause_agents(reg, &row.worktree) } else { vec![] };
        let at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
        budget::push_event(row, &paused, at);
    }
    if state != before {
        let _ = write_json(&path, &state);
    }
    reg.announce_events();
    budget::status(&state, &budgets, &totals, date)
}
