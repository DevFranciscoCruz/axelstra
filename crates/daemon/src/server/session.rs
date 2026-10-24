use super::*;

pub(super) fn call(c: &mut Ctx) -> Result<Value> {
    let req = c.req;
    let (p, reg, tx) = (c.p, c.reg, c.tx);
    let subs = &mut *c.subs;
    let after = &mut *c.after;
    let sid = || -> Result<u32> { p.get("id").and_then(Value::as_u64).and_then(|x| u32::try_from(x).ok()).ok_or_else(|| missing("id")) };
    let session = || -> Result<Arc<Session>> {
        let id = sid()?;
        reg.get(id).ok_or_else(|| axelstra_core::coded!("session.not_found", "no such session {id}", id = id))
    };
    match req.method.as_str() {
        "session.spawn" => {
            let mut s = Spawn {
                program: p["program"].as_str().ok_or_else(|| missing("program"))?.into(),
                args: p["args"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default(),
                cwd: p["cwd"].as_str().map(Into::into),
                env: p["env"]
                    .as_object()
                    .map(|o| o.iter().filter_map(|(k, v)| v.as_str().map(|v| (k.clone(), v.to_string()))).collect())
                    .unwrap_or_default(),
                cols: term_dim(&p["cols"], 120),
                rows: term_dim(&p["rows"], 30),
            };
            let mut meta = if p["meta"].is_null() { None } else { Some(p["meta"].clone()) };
            // a session started on behalf of another one (a worker): remembered as its child
            let mut parent = p["parent"].as_u64().or_else(|| p["reap"]["parent"].as_u64()).map(|x| x as u32);
            // the lead of a worktree: its first agent. Tasks started there report to it; it reports to the project main
            let (lead, parent_default) = {
                let mut m = meta.clone().unwrap_or(Value::Null);
                // a dispatched lead says so itself
                if p["reap"]["lead"].as_bool() == Some(true) && m.is_object() {
                    m["lead"] = json!(true);
                }
                let (project, wt) = (
                    m["project"].as_str().unwrap_or_default().to_string(),
                    axelstra_core::paths::norm(m["worktree"].as_str().unwrap_or_default()),
                );
                let sessions = reg.sessions.lock_recover();
                let metas = reg.metas.lock_recover();
                let live = |id: &u32| sessions.get(id).is_some_and(|s| s.try_wait().is_none());
                let mut here = Vec::new();
                let mut project_main = None;
                for (id, o) in
                    metas.iter().filter(|(id, o)| o["kind"] == "agent" && o["project"].as_str() == Some(project.as_str()) && live(id))
                {
                    if o["main"] == true {
                        project_main = project_main.or(Some(*id));
                    } else if axelstra_core::paths::norm(o["worktree"].as_str().unwrap_or_default()) == wt {
                        here.push((*id, o["lead"] == true));
                    }
                }
                let worker = p["worker"].as_bool() == Some(true);
                crate::notes::pick_role(&m, p["label"].as_str(), worker, &here, project_main, parent)
            };
            if lead {
                meta.get_or_insert_with(|| json!({}))["lead"] = json!(true);
            }
            parent = parent_default;
            // a dispatched lead is ephemeral like a worker but owns its worktree: it does not count against the worker limit
            let is_worker =
                !lead && ((p["reap"].is_object() && p["reap"]["lead"].as_bool() != Some(true)) || p["worker"].as_bool() == Some(true));
            if is_worker {
                let alive = |i: u32| reg.get(i).map(|s| s.try_wait().is_none()).unwrap_or(false);
                let (live, max) = (reg.bus.live_workers(alive), reg.bus.limit());
                if live >= max {
                    return Err(axelstra_core::coded!(
                        "session.worker_limit",
                        "worker limit reached ({live} running, max {max}): wait for one to finish or raise it with `axelstra limit N`",
                        live = live,
                        max = max
                    ));
                }
            }
            if let Some(par) = parent {
                let m = meta.get_or_insert_with(|| json!({}));
                m["parent"] = json!(par);
                if is_worker {
                    m["worker"] = json!(true);
                }
                s.env.push(("AXELSTRA_PARENT".into(), par.to_string()));
            }
            let history_key = p["meta"]["key"].as_str().map(String::from);
            // an agent gets a name ("Tarefa N") and its task file; the prompt it starts with is the goal
            let data_dir = reg.saved.parent().unwrap_or(std::path::Path::new("")).to_path_buf();
            // no other agent of the project running: its numbering starts again
            let alone = {
                let project = meta.as_ref().and_then(|m| m["project"].as_str()).unwrap_or_default().to_string();
                let sessions = reg.sessions.lock_recover();
                let metas = reg.metas.lock_recover();
                !metas.iter().any(|(id, m)| {
                    m["kind"] == "agent"
                        && m["main"] != true
                        && m["project"].as_str() == Some(project.as_str())
                        && sessions.get(id).is_some_and(|s| s.try_wait().is_none())
                })
            };
            let label = crate::notes::on_spawn(&crate::notes::root(&data_dir), p["label"].as_str(), &mut meta, p["goal"].as_str(), alone);
            let note = meta.as_ref().and_then(|m| m.get("note").cloned());
            let id = reg.spawn(&s, label.as_deref(), meta)?;
            // a restored agent shows what its terminal said before the restart, dimmed, above its own output
            if let (Some(key), Some(sess)) = (history_key, reg.get(id)) {
                if let Some(text) = take_history(reg, &key) {
                    sess.seed(text.as_bytes());
                }
            }
            if let Some(par) = parent {
                reg.bus.set_parent(id, par);
            }
            if is_worker {
                reg.bus.add_worker(id);
            }
            if let Some(r) = p["reap"].as_object() {
                // ephemeral worker: never restored after a restart, reports to `parent` and closes itself
                reg.bus.mark_reaped(id);
                let parent = r.get("parent").and_then(Value::as_u64).map(|x| x as u32);
                let idle_mode = r.get("mode").and_then(Value::as_str) == Some("idle");
                let idle_secs = r.get("idle_secs").and_then(Value::as_u64).unwrap_or(20).max(3);
                let lines = r.get("lines").and_then(Value::as_u64).unwrap_or(40).clamp(1, 400) as usize;
                spawn_reaper(reg.clone(), id, parent, idle_mode, idle_secs, lines);
                return Ok(json!({"id": id}));
            }
            let mut spec = p.clone();
            spec["id"] = Value::Null;
            // restoring brings back the name and the file; the goal text stays out of sessions.json
            spec.as_object_mut().map(|o| o.remove("goal"));
            if let Some(l) = &label {
                spec["label"] = json!(l);
            }
            if let (Some(n), true) = (note, spec["meta"].is_object()) {
                spec["meta"]["note"] = n;
            }
            // restoring brings the lead back as the lead
            if lead && spec["meta"].is_object() {
                spec["meta"]["lead"] = json!(true);
            }
            reg.specs.lock_recover().insert(id, spec);
            reg.persist();
            Ok(json!({"id": id}))
        }
        "session.list" => Ok(Value::Array(session_list(reg))),
        // an agent's hook (`axelstra hook`) says what it is doing: waiting, idle or working
        "session.hint" => {
            let state = axelstra_core::waiting::HintState::parse(p["state"].as_str().unwrap_or_default())
                .ok_or_else(|| axelstra_core::coded!("session.bad_state", "state must be waiting, idle or working"))?;
            session()?;
            reg.hints.set(sid()?, state);
            Ok(json!({}))
        }
        "session.subscribe" => {
            let id = sid()?;
            let s = session()?;
            if let Some(old) = subs.remove(&id) {
                old.store(true, Ordering::SeqCst);
            }
            let stop = Arc::new(AtomicBool::new(false));
            subs.insert(id, stop.clone());
            let (snap, rx) = s.subscribe();
            let tx = tx.clone();
            // plain thread: the PTY side is blocking std channels
            after.push(Box::new(move || {
                std::thread::spawn(move || {
                    if !snap.is_empty() {
                        let _ = tx.blocking_send(session_frame(OUTPUT, id, &snap));
                    }
                    loop {
                        if stop.load(Ordering::SeqCst) {
                            return;
                        }
                        // blocks until the session prints or its stream ends (`watch_exit` ends it when the program
                        // is gone, even if ConPTY keeps the pipe open); the 5 s wake-up is the heartbeat below
                        match rx.recv_timeout(STREAM_HEARTBEAT) {
                            Ok(d) => {
                                if tx.blocking_send(session_frame(OUTPUT, id, &d)).is_err() {
                                    return;
                                }
                            }
                            Err(RecvTimeoutError::Timeout) => {
                                // an empty frame every 5 quiet seconds: a client that stopped listening (its tab closed) wakes up,
                                // sees it is detached and hangs up, instead of staying blocked until this session prints again
                                if tx.blocking_send(session_frame(OUTPUT, id, &[])).is_err() {
                                    return;
                                }
                                if s.try_wait().is_some() {
                                    while let Ok(d) = rx.recv_timeout(Duration::from_millis(150)) {
                                        let _ = tx.blocking_send(session_frame(OUTPUT, id, &d));
                                    }
                                    break;
                                }
                            }
                            Err(RecvTimeoutError::Disconnected) => break,
                        }
                    }
                    let mut code = None;
                    for _ in 0..40 {
                        code = s.try_wait();
                        if code.is_some() {
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(50));
                    }
                    let ev = json!({"event": "session.exit", "id": id, "code": code});
                    let _ = tx.blocking_send(encode(EVENT, ev.to_string().as_bytes()));
                });
            }));
            Ok(json!({}))
        }
        "session.unsubscribe" => {
            if let Some(stop) = subs.remove(&sid()?) {
                stop.store(true, Ordering::SeqCst);
            }
            Ok(json!({}))
        }
        "session.resize" => {
            session()?.resize(term_dim(&p["cols"], 120), term_dim(&p["rows"], 30))?;
            Ok(json!({}))
        }
        "session.status" => {
            let code = session()?.try_wait();
            Ok(json!({"running": code.is_none(), "code": code}))
        }
        // the tab name the user chose: kept with the session (and in sessions.json, so a restore reopens it under that name)
        "session.rename" => {
            let id = sid()?;
            session()?;
            let wanted: String = p["label"].as_str().unwrap_or_default().trim().chars().take(120).collect();
            let mut labels = reg.labels.lock_recover();
            let mut specs = reg.specs.lock_recover();
            // the name the session was started with, kept the first time it is renamed, so an empty name can bring it back
            let base = specs.get(&id).and_then(|sp| sp["base_label"].as_str().map(str::to_string)).or_else(|| labels.get(&id).cloned());
            let label = if wanted.is_empty() { base.clone().unwrap_or_default() } else { wanted };
            if label.is_empty() {
                labels.remove(&id);
            } else {
                labels.insert(id, label.clone());
            }
            if let Some(spec) = specs.get_mut(&id) {
                if spec.get("base_label").is_none() {
                    if let Some(b) = &base {
                        spec["base_label"] = json!(b);
                    }
                }
                if label.is_empty() {
                    spec.as_object_mut().map(|o| o.remove("label"));
                } else {
                    spec["label"] = json!(label);
                }
            }
            drop(labels);
            drop(specs);
            reg.persist();
            Ok(json!({"label": label}))
        }
        "session.remove" => {
            let id = sid()?;
            if let Some(stop) = subs.remove(&id) {
                stop.store(true, Ordering::SeqCst);
            }
            if let Some(s) = reg.sessions.lock_recover().remove(&id) {
                s.kill();
            }
            reg.labels.lock_recover().remove(&id);
            // a closed tab keeps no history: the periodic save would otherwise leave its file behind
            if let Some(f) = reg.metas.lock_recover().remove(&id).and_then(|m| m["key"].as_str().and_then(|k| history_file(reg, k))) {
                let _ = std::fs::remove_file(f);
            }
            reg.specs.lock_recover().remove(&id);
            reg.bus.forget(id);
            reg.persist();
            Ok(json!({}))
        }
        "session.pending" => {
            let sid_dir = reg.saved.with_file_name("sid");
            let mut v = reg.pending.lock_recover().clone();
            for rec in v.iter_mut() {
                // the agent may have started a new conversation (/clear) since: resume the latest one
                let Some(key) = rec["meta"]["key"].as_str().map(String::from) else { continue };
                let latest = std::fs::read_to_string(sid_dir.join(format!("{key}.json")))
                    .ok()
                    .and_then(|t| serde_json::from_str::<Value>(&t).ok())
                    .and_then(|j| j["session_id"].as_str().map(String::from));
                if let Some(id) = latest {
                    // a conversation with no replies (cleared, never used) is not worth resuming: start clean
                    let cwd = rec["cwd"].as_str().or(rec["meta"]["worktree"].as_str()).unwrap_or("");
                    // only a conversation file that exists and has no replies counts as empty; a file that cannot be found
                    // (another config dir, a different path spelling) is resumed and, failing that, started fresh
                    let empty = crate::agentlog::claude_usage(cwd, Some(&id)).map(|u| u.messages == 0).unwrap_or(false);
                    if empty {
                        rec["meta"]["blank"] = Value::Bool(true);
                    }
                    rec["meta"]["resume"] = Value::String(id);
                }
            }
            Ok(Value::Array(v))
        }
        "session.pending_remove" => {
            // one restored (or deliberately skipped) entry leaves the list; the rest wait, so a failure midway loses nothing
            let rec = &p["rec"];
            let same = |a: &Value| match (a["meta"]["key"].as_str(), rec["meta"]["key"].as_str()) {
                (Some(x), Some(y)) => x == y,
                _ => a["program"] == rec["program"] && a["cwd"] == rec["cwd"] && a["label"] == rec["label"],
            };
            let mut list = reg.pending.lock_recover();
            if let Some(i) = list.iter().position(same) {
                list.remove(i);
            }
            drop(list);
            reg.persist();
            Ok(json!({}))
        }
        "session.pending_clear" => {
            reg.pending.lock_recover().clear();
            reg.persist();
            Ok(json!({}))
        }
        "session.kill" => {
            session()?.kill();
            Ok(json!({}))
        }
        // the last lines as the terminal shows them (replayed through an emulator), for `axelstra read` and flow reports
        "session.screen" => {
            let s = session()?;
            let n = p["lines"].as_u64().unwrap_or(40).clamp(1, 500) as usize;
            Ok(json!({"lines": tokio::task::block_in_place(|| s.screen_lines(n))}))
        }
        "session.resources" => {
            // memory now and total CPU time of every live session (the caller turns CPU time into a percentage)
            let l = reg
                .sessions
                .lock_recover()
                .iter()
                .filter(|(_, s)| s.try_wait().is_none())
                .map(|(id, s)| (*id, s.clone()))
                .collect::<Vec<_>>();
            let snap = axelstra_core::procs::snapshot_cached(Duration::from_millis(800));
            let v: Vec<Value> =
                l.iter().map(|(id, s)| json!({"id": id, "mem": s.memory_now_in(&snap), "cpu_100ns": s.job_usage_in(&snap).1})).collect();
            Ok(json!({"sessions": v, "cores": std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1)}))
        }
        "session.usage" => {
            let (mem, cpu) = session()?.job_usage();
            Ok(json!({"peak_mem_bytes": mem, "cpu_100ns": cpu}))
        }
        m => anyhow::bail!("unknown method {m}"),
    }
}
