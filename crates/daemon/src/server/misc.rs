use super::*;

pub(super) fn call(c: &mut Ctx) -> Result<Value> {
    let req = c.req;
    let (p, reg) = (c.p, c.reg);
    let (authed, cli, agent) = (&mut *c.authed, &mut *c.cli, &mut *c.agent);
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
                anyhow::bail!("bad token");
            }
            reg.bad_hellos.store(0, Ordering::SeqCst);
            *authed = true;
            *agent = agent_session;
            *cli = agent_session.is_some() || p.get("client").and_then(Value::as_str) == Some("cli");
            Ok(hello_info())
        }
        "agent.status" => {
            let key = p["key"].as_str().unwrap_or_default();
            if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
                anyhow::bail!("bad key");
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
            Ok(json!({}))
        }
        "flow.list" => {
            let all: Vec<Value> = reg.flows.lock_recover().values().flat_map(|t| t.as_array().cloned().unwrap_or_default()).collect();
            Ok(json!({"tasks": all}))
        }
        "agents.list" => Ok(json!({"agents": tokio::task::block_in_place(axelstra_core::agents::list)})),
        "codex.limits" => Ok(json!({"limits": tokio::task::block_in_place(crate::agentlog::codex_limits)})),
        "computer.status" => Ok(json!({"enabled": reg.computer.load(Ordering::SeqCst)})),
        "computer.enable" => {
            // agents run the CLI: only the app (a connection that did not say it is the CLI) may turn it on
            if *cli && p["on"].as_bool() == Some(true) {
                anyhow::bail!("screen control can only be turned on by the user in the app");
            }
            reg.computer.store(p["on"].as_bool() == Some(true), Ordering::SeqCst);
            Ok(json!({"enabled": reg.computer.load(Ordering::SeqCst)}))
        }
        "app.dir" => Ok(json!({"dir": reg.saved.parent().unwrap_or(std::path::Path::new("")).to_string_lossy().replace('\\', "/")})),
        "settings.get" => {
            let saved = read_settings(&reg.saved);
            // `terse` in the request asks for the rules of that level instead of the saved one (a project's own choice)
            let terse = p["terse"].as_str().or(saved["terse"].as_str()).filter(|l| ["off", "short", "tiny"].contains(l)).unwrap_or("off");
            let installed = tokio::task::block_in_place(|| crate::projects::package_managers(None, None).0);
            let pm = saved["pm"].as_str().filter(|m| installed.contains(m)).map(String::from);
            let (shell, notify_sound, notify_mode) = ui_settings(&saved);
            Ok(
                json!({"terse": terse, "rules": crate::skill::terse_rules(terse), "pm": pm, "installed": installed, "auto_install": saved["auto_install"] == json!(true),
                "shell": shell, "notify_sound": notify_sound, "notify_mode": notify_mode}),
            )
        }
        "settings.set" => {
            // partial update: only the keys present are changed
            let mut saved = read_settings(&reg.saved);
            if let Some(level) = p["terse"].as_str() {
                if !["off", "short", "tiny"].contains(&level) {
                    return Err(anyhow::anyhow!("terse must be off, short or tiny"));
                }
                saved["terse"] = json!(level);
            }
            if p.get("pm").is_some() {
                saved["pm"] = match p["pm"].as_str() {
                    Some(m) if ["bun", "pnpm", "yarn", "npm"].contains(&m) => json!(m),
                    Some(_) => return Err(anyhow::anyhow!("pm must be bun, pnpm, yarn or npm")),
                    None => Value::Null,
                };
            }
            if let Some(b) = p["auto_install"].as_bool() {
                saved["auto_install"] = json!(b);
            }
            apply_ui_settings(&mut saved, p)?;
            write_json(&reg.saved.with_file_name("settings.json"), &saved)?;
            Ok(json!({}))
        }
        "agent.search" => {
            let q = p["query"].as_str().unwrap_or_default().to_string();
            let max = p["max"].as_u64().unwrap_or(30).clamp(1, 200) as usize;
            let hits = tokio::task::block_in_place(|| crate::agentlog::search(&q, max));
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
        "hooks.status" => Ok(crate::hooks::status()?),
        "hooks.install" => tokio::task::block_in_place(crate::hooks::install),
        "ping" => Ok(hello_info()),
        // what the daemon recovered or lost on its own (a state file moved aside, restored from a backup): shown once
        "audit.list" => {
            let limit = p["limit"].as_u64().map_or(100, |n| n.clamp(1, 500) as usize);
            let session = p["session"].as_u64().and_then(|x| u32::try_from(x).ok());
            Ok(json!({"entries": reg.audit.list(limit, session)}))
        }
        "notices.take" => Ok(json!({"notices": crate::store::take_notices()})),
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
            reg.persist();
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
