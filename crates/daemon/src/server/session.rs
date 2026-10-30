use super::*;

/// What a terminal stream's forwarder waits on: output from the pump, the end of the stream, or a nudge to look at
/// its stop flag and its connection.
pub(super) enum Stream {
    Data(Vec<u8>),
    End,
    Wake,
}

/// Chunks waiting between a stream's pump and its forwarder (the PTY side already drops output for a slow subscriber).
const STREAM_QUEUE: usize = 4096;

type StreamWaker = (Arc<AtomicBool>, std::sync::mpsc::SyncSender<Stream>);

/// Running terminal streams: the stop flag and the channel that wakes the forwarder.
fn streams() -> &'static Mutex<Vec<StreamWaker>> {
    static W: std::sync::OnceLock<Mutex<Vec<StreamWaker>>> = std::sync::OnceLock::new();
    W.get_or_init(|| Mutex::new(Vec::new()))
}

pub(super) fn register_stream(stop: &Arc<AtomicBool>, tx: &std::sync::mpsc::SyncSender<Stream>) {
    streams().lock_recover().push((stop.clone(), tx.clone()));
}

pub(super) fn unregister_stream(stop: &Arc<AtomicBool>) {
    streams().lock_recover().retain(|(s, _)| !Arc::ptr_eq(s, stop));
}

/// How many terminal streams are forwarding (tests: a dead client's stream must leave this by itself).
pub(super) fn live_streams() -> usize {
    streams().lock_recover().len()
}

/// Call after a stop flag was set or a connection's writer ended: every forwarder looks at its flag and its
/// connection and ends if the client is gone. A full queue means it has chunks to read and checks on its way.
pub(super) fn wake_streams() {
    for (_, tx) in streams().lock_recover().iter() {
        let _ = tx.try_send(Stream::Wake);
    }
}

pub(super) fn call(c: &mut Ctx) -> Result<Value> {
    let req = c.req;
    let (p, reg, tx) = (c.p, c.reg, c.tx);
    let subs = &mut *c.subs;
    let after = &mut *c.after;
    // an agent connection reaches only its own session and the ones it started (the app and the user's CLI reach all)
    let agent: Option<u32> = *c.agent;
    let own_tree = |id: u32| -> Result<()> {
        match agent {
            Some(me) if !agent_drives(reg, me, id) => Err(not_own_tree(&req.method)),
            _ => Ok(()),
        }
    };
    let sid = || -> Result<u32> { p.get("id").and_then(Value::as_u64).and_then(|x| u32::try_from(x).ok()).ok_or_else(|| missing("id")) };
    let session = || -> Result<Arc<Session>> {
        let id = sid()?;
        reg.get(id).ok_or_else(|| {
            if reg.hib.is_sleeping(id) || reg.hib.is_waking(id).is_some() {
                axelstra_core::coded!("hibernate.sleeping", "session {id} is hibernated: wake it with `axelstra wake {id}`", id = id)
            } else {
                axelstra_core::coded!("session.not_found", "no such session {id}", id = id)
            }
        })
    };
    match req.method.as_str() {
        "session.hibernate" | "session.wake" => hibernate::call(c),
        "session.spawn" => {
            // idempotent: a restore that races another one (or a retry) must not start the same conversation twice
            if let Some(id) = p["meta"]["key"].as_str().and_then(|k| reg.live_by_key(k)) {
                return Ok(json!({"id": id, "existing": true}));
            }
            // a low disk starts reclaiming build artifacts of inactive worktrees (in a thread: the spawn never waits)
            tokio::task::block_in_place(|| super::disk::ensure(reg, false));
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
            // the main checkout never has tasks, it only dispatches them: an agent or the CLI cannot start one there
            if agent.is_some() || p["from"] == "cli" {
                main_checkout_guard(reg, p)?;
            }
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
                for (id, o) in metas.iter().filter(|(id, o)| {
                    o["kind"] == "agent" && o["main"] != true && o["project"].as_str() == Some(project.as_str()) && live(id)
                }) {
                    if axelstra_core::paths::norm(o["worktree"].as_str().unwrap_or_default()) == wt {
                        here.push((*id, o["lead"] == true));
                    }
                }
                let project_main = project_main_in(&sessions, &metas, &project);
                let worker = p["worker"].as_bool() == Some(true);
                crate::notes::pick_role(&m, worker, &here, project_main, parent)
            };
            if lead {
                meta.get_or_insert_with(|| json!({}))["lead"] = json!(true);
            }
            parent = parent_default;
            // a dispatched lead is ephemeral like a worker but owns its worktree: it does not count against the worker limit
            let is_worker =
                !lead && ((p["reap"].is_object() && p["reap"]["lead"].as_bool() != Some(true)) || p["worker"].as_bool() == Some(true));
            // `queue: true`: a refusal for the worker limit or for memory waits in line (FIFO) instead of failing. The call
            // sleeps on the pulse (a freed slot, a limit change, a memory level change) up to one chunk, then answers
            // `{queued, ticket, position}`; the caller comes back with its ticket and keeps its place
            let queue = p["queue"].as_bool() == Some(true);
            let mut ticket: Option<u64> = None;
            // the place in line is given up when this call ends, not when the check passes: the session must be in
            // `sessions` first, or the next in line would count a free slot that is already taken
            let mut place = LinePlace { reg: reg.clone(), ticket: None };
            let chunk_end = Instant::now() + SPAWN_QUEUE_CHUNK;
            loop {
                // taken before the check, so a pulse between the check and the sleep is not lost
                let seen = reg.pulse.now();
                if queue && (p["ticket"].is_u64() || !reg.spawnq.lock_recover().is_empty()) {
                    ticket = Some(reg.spawnq.lock_recover().enter(ticket.or(p["ticket"].as_u64()), Instant::now()));
                    place.ticket = ticket;
                }
                let refusal: Result<()> = (|| {
                    if let Some(t) = ticket {
                        if !reg.spawnq.lock_recover().is_head(t, Instant::now()) {
                            return Err(axelstra_core::coded!("session.queued", "waiting in line for a worker slot"));
                        }
                    }
                    {
                        // memory pressure: an agent started by an agent waits, and the worker limit drops (High: half, Critical: 1)
                        let alive = |i: u32| reg.get(i).map(|s| s.try_wait().is_none()).unwrap_or(false);
                        let kind = crate::governor::SpawnKind {
                            worker: is_worker,
                            by_agent: agent.is_some(),
                            agent: meta.as_ref().is_some_and(|m| m["kind"] == "agent"),
                            live_workers: if is_worker { reg.bus.live_workers(alive) } else { 0 },
                        };
                        let (lv, info) = reg.governor.state();
                        let (cfg, limit) = (reg.governor.config(), reg.bus.limit());
                        if crate::governor::admission_denied(&cfg, lv, limit, kind) {
                            return Err(axelstra_core::coded!(
                                "governor.admission_denied",
                                "not started: memory is {level} ({free_mb} MB free). Agents wait until it recovers; workers are limited to {max}",
                                level = lv.as_str(),
                                free_mb = info.free_mb,
                                max = crate::governor::effective_worker_limit(limit, lv)
                            ));
                        }
                    }
                    if is_worker {
                        let alive = |i: u32| reg.get(i).map(|s| s.try_wait().is_none()).unwrap_or(false);
                        let (live, max) = (reg.bus.live_workers(alive), reg.bus.limit());
                        if live >= max {
                            // the caller waits for a slot: that is work, not silence, for the stall watchdog
                            if let Some(me) = agent {
                                reg.work.want_slot(me);
                                reg.pulse.bump();
                            }
                            return Err(axelstra_core::coded!(
                                "session.worker_limit",
                                "worker limit reached ({live} running, max {max}): wait for one to finish or raise it with `axelstra limit N`",
                                live = live,
                                max = max
                            ));
                        }
                        if let Some(me) = agent {
                            reg.work.got_slot(me);
                        }
                    }
                    Ok(())
                })();
                match refusal {
                    Ok(()) => break,
                    Err(e) if queue && is_queueable(&e) => {
                        let t = *ticket.get_or_insert_with(|| reg.spawnq.lock_recover().enter(None, Instant::now()));
                        place.ticket = ticket;
                        let now = Instant::now();
                        if now >= chunk_end || reg.closing.load(Ordering::SeqCst) {
                            let position = reg.spawnq.lock_recover().position(t);
                            let reason = axelstra_core::msg::find(&e).map(|c| c.code.clone());
                            // the caller comes back with the ticket: the place stays
                            place.ticket = None;
                            return Ok(json!({"queued": true, "ticket": t, "position": position, "reason": reason}));
                        }
                        reg.pulse.wait(seen, Some(chunk_end - now));
                    }
                    Err(e) => return Err(e),
                }
            }
            // the flag stays in the meta so a hibernated worker counts against the limit again when it wakes
            if is_worker {
                meta.get_or_insert_with(|| json!({}))["worker"] = json!(true);
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
            // a lead started by hand with no prompt, no dispatch and no conversation to resume has no request yet: the stall
            // watchdog leaves it alone until someone writes to it. Live meta only; `spec` below is cut from `p`, so a
            // restart does not keep the flag
            if lead && p["goal"].as_str().is_none_or(|g| g.trim().is_empty()) && !p["reap"].is_object() && p["meta"]["note"].is_null() {
                if let Some(m) = meta.as_mut().filter(|m| m.is_object()) {
                    m["unrequested"] = json!(true);
                }
            }
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
            // a dispatched lead or a worker is ephemeral only in that it closes itself after `done`: while it runs and has not
            // reported, a restart must bring it back (as a plain agent: the reaper is not restored), so `reap` stays out
            let mut spec = p.clone();
            spec["id"] = Value::Null;
            spec.as_object_mut().map(|o| o.remove("reap"));
            // a hibernated worker or lead is reaped again when it wakes
            reg.hib.note_reap(id, p["reap"].clone());
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
            // a reaped plain shell (a test runner, a one-off command) is not worth bringing back; a reaped agent is
            if p["reap"].is_null() || spec["meta"]["kind"] == "agent" {
                reg.specs.lock_recover().insert(id, spec);
                reg.persist();
            }
            if let Some(r) = p["reap"].as_object() {
                // ephemeral worker or lead: reports to `parent` and closes itself (its spec, if an agent, was saved above)
                reg.bus.mark_reaped(id);
                let parent = r.get("parent").and_then(Value::as_u64).map(|x| x as u32);
                let idle_mode = r.get("mode").and_then(Value::as_str) == Some("idle");
                let idle_secs =
                    r.get("idle_secs").and_then(Value::as_u64).unwrap_or(20).max(reaper_min_idle_secs(p["meta"]["kind"] == "agent"));
                let lines = r.get("lines").and_then(Value::as_u64).unwrap_or(40).clamp(1, 400) as usize;
                spawn_reaper(reg.clone(), id, parent, idle_mode, idle_secs, lines);
                return Ok(json!({"id": id}));
            }
            Ok(json!({"id": id}))
        }
        "session.list" => Ok(Value::Array(session_list(reg))),
        // an agent's hook (`axelstra hook`) says what it is doing: waiting, idle or working. A wait carries its kind
        // (permission, question, idle_input) and the agent's own text; without a kind it is not a wait.
        "session.hint" => {
            let state = axelstra_core::waiting::HintState::parse(p["state"].as_str().unwrap_or_default())
                .ok_or_else(|| axelstra_core::coded!("session.bad_state", "state must be waiting, idle or working"))?;
            session()?;
            let id = sid()?;
            // only the agent itself says it waits
            require_own(*c.agent, Some(u64::from(id)), &req.method)?;
            let wait = p["kind"].as_str().and_then(axelstra_core::waiting::WaitKind::parse).map(|kind| axelstra_core::waiting::Wait {
                kind,
                text: axelstra_core::waiting::clean_text(p["text"].as_str().unwrap_or_default()),
            });
            // the permission mode the agent reports (additive; an old hook sends none)
            if let Some(m) = p["mode"].as_str().map(axelstra_core::waiting::clean_text).filter(|m| !m.is_empty() && m.len() <= 40) {
                reg.modes.lock_recover().insert(id, m);
            }
            reg.hints.set(id, state, wait);
            reg.tell_parent_waiting(id);
            Ok(json!({}))
        }
        // an agent says it only waits for something external (CI, a deploy): the stall watchdog stays quiet until it
        // works again, the user types, `done`, `clear` or the limit (`for_min`, at most 2 h) passes
        "session.await" => {
            session()?;
            let id = sid()?;
            require_own(*c.agent, Some(u64::from(id)), &req.method)?;
            if p["clear"].as_bool() == Some(true) {
                reg.awaits.clear(id);
            } else {
                let reason = axelstra_core::waiting::clean_text(p["reason"].as_str().unwrap_or_default());
                if reason.is_empty() {
                    return Err(axelstra_core::coded!("session.await_reason", "an await needs a reason, e.g. \"CI #3703\""));
                }
                reg.awaits.set(id, &reason, p["for_min"].as_u64());
            }
            reg.pulse.bump();
            Ok(json!({}))
        }
        "session.subscribe" => {
            let id = sid()?;
            // opening a hibernated tab wakes it; the stream starts once its process exists
            if reg.hib.is_sleeping(id) {
                tokio::task::block_in_place(|| hibernate::wake_wait(reg, id, false, Duration::from_secs(20)));
            }
            let s = session()?;
            own_tree(id)?;
            if let Some(old) = subs.remove(&id) {
                old.store(true, Ordering::SeqCst);
                wake_streams();
            }
            let stop = Arc::new(AtomicBool::new(false));
            subs.insert(id, stop.clone());
            let (snap, rx) = s.subscribe();
            let tx = tx.clone();
            // plain thread: the PTY side is blocking std channels
            after.push(Box::new(move || {
                // The PTY's receiver cannot be woken from outside, so a pump thread owns it (it holds no `tx`) and the
                // forwarder waits on a local channel that `wake_streams` can also post to. A dead client (stop flag set,
                // or the connection's writer gone so `tx` is closed) wakes the forwarder at once: it drops `tx` and ends,
                // with no heartbeat. The pump only lingers until the session prints or ends, then sees the closed channel.
                let (ltx, lrx) = std::sync::mpsc::sync_channel::<Stream>(STREAM_QUEUE);
                register_stream(&stop, &ltx);
                std::thread::spawn(move || {
                    // blocks until the session prints or its stream ends (`watch_exit` ends it when the program
                    // is gone, even if ConPTY keeps the pipe open)
                    loop {
                        match rx.recv() {
                            Ok(d) => {
                                if ltx.send(Stream::Data(d)).is_err() {
                                    return;
                                }
                            }
                            Err(_) => {
                                let _ = ltx.send(Stream::End);
                                return;
                            }
                        }
                    }
                });
                std::thread::spawn(move || {
                    let gone = || stop.load(Ordering::SeqCst) || tx.is_closed();
                    if !snap.is_empty() && tx.blocking_send(session_frame(OUTPUT, id, &snap)).is_err() {
                        unregister_stream(&stop);
                        return;
                    }
                    loop {
                        if gone() {
                            unregister_stream(&stop);
                            return;
                        }
                        match lrx.recv() {
                            Ok(Stream::Data(d)) => {
                                if tx.blocking_send(session_frame(OUTPUT, id, &d)).is_err() {
                                    unregister_stream(&stop);
                                    return;
                                }
                            }
                            Ok(Stream::Wake) => {}
                            Ok(Stream::End) => break,
                            Err(_) => {
                                unregister_stream(&stop);
                                return;
                            }
                        }
                    }
                    unregister_stream(&stop);
                    // the stream only ends once the program is gone (`watch_exit`), so the OS exit signal is already there
                    s.wait_exit();
                    let code = s.try_wait();
                    let ev = json!({"event": "session.exit", "id": id, "code": code});
                    let _ = tx.blocking_send(encode(EVENT, ev.to_string().as_bytes()));
                });
            }));
            Ok(json!({}))
        }
        "session.unsubscribe" => {
            if let Some(stop) = subs.remove(&sid()?) {
                stop.store(true, Ordering::SeqCst);
                wake_streams();
            }
            Ok(json!({}))
        }
        "session.resize" => {
            session()?.resize(term_dim(&p["cols"], 120), term_dim(&p["rows"], 30))?;
            Ok(json!({}))
        }
        "session.status" => {
            // hibernated: not ended, there is only no process now
            if sid().is_ok_and(|id| reg.hib.is_sleeping(id) || (reg.hib.is_waking(id).is_some() && reg.get(id).is_none())) {
                return Ok(json!({"running": true, "code": null, "hibernated": true}));
            }
            // one that ended on its own already left the registry: its code is still answered (`gone`)
            if let Some((code, finished)) = sid().ok().filter(|id| reg.get(*id).is_none()).and_then(|id| reg.exited_of(id)) {
                let wake_error = sid().ok().and_then(|id| reg.hib.failed(id));
                return Ok(json!({"running": false, "code": code, "gone": true, "finished": finished, "wake_error": wake_error}));
            }
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
            own_tree(id)?;
            if let Some(stop) = subs.remove(&id) {
                stop.store(true, Ordering::SeqCst);
                wake_streams();
            }
            reg.resume_before_kill(id);
            let removed = reg.sessions.lock_recover().remove(&id);
            if let Some(s) = removed {
                s.kill();
            }
            reg.labels.lock_recover().remove(&id);
            // a closed tab keeps no history: the periodic save would otherwise leave its file behind
            if let Some(f) = reg.metas.lock_recover().remove(&id).and_then(|m| m["key"].as_str().and_then(|k| history_file(reg, k))) {
                let _ = std::fs::remove_file(f);
            }
            let gone = reg.specs.lock_recover().remove(&id);
            if let Some(spec) = gone {
                reg.drop_sid_of(&spec);
            }
            reg.bus.forget(id);
            reg.hib.forget(id);
            reg.persist();
            Ok(json!({}))
        }
        "session.pending" => {
            let sid_dir = reg.saved.with_file_name("sid");
            // entries whose worktree was deleted meanwhile are dropped; every other one is handed out, none is dropped by a cap
            let mut v: Vec<Value> = reg.pending.lock_recover().clone();
            let before = v.len();
            // a task that reported `done` before the restart is not brought back either
            // a bare shell (no folder, no meta) is junk an older run saved: never handed out
            v.retain(|r| rec_alive(r) && !rec_junk(r) && r["meta"]["done"] != true);
            if v.len() != before {
                *reg.pending.lock_recover() = v.clone();
                reg.persist();
            }
            for rec in v.iter_mut() {
                // the agent may have started a new conversation (/clear) since: resume the latest one
                let Some(key) = rec["meta"]["key"].as_str().map(String::from) else { continue };
                let hook =
                    std::fs::read_to_string(sid_dir.join(format!("{key}.json"))).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok());
                let latest = hook.as_ref().and_then(|j| j["session_id"].as_str().filter(|s| valid_resume_id(s)).map(String::from));
                // Claude writes the transcript with the first prompt: the hook names the file, and a file that is not there
                // means the agent never had a request (a fresh lead nobody briefed). Nothing to bring back.
                if hook.as_ref().is_some_and(never_prompted) {
                    rec["meta"]["blank"] = Value::Bool(true);
                }
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
            // an entry whose conversation already runs is not handed out again (it stays saved: the live spec replaces it)
            v.retain(|r| r["meta"]["key"].as_str().and_then(|k| reg.live_by_key(k)).is_none());
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
            // a hibernated session has no process: closing its tab drops the row and what was kept for it
            if sid().is_ok_and(|id| reg.hib.is_sleeping(id)) {
                let id = sid()?;
                own_tree(id)?;
                reg.forget_session(id);
                reg.persist();
                reg.pulse.bump();
                reg.broadcast("session.changed");
                return Ok(json!({}));
            }
            let s = session()?;
            let id = sid()?;
            own_tree(id)?;
            reg.resume_before_kill(id);
            s.kill();
            // a killed session leaves the list and sessions.json right away (the exit watcher only ends its stream)
            reg.forget_session(id);
            reg.persist();
            reg.pulse.bump();
            Ok(json!({}))
        }
        // the last lines as the terminal shows them (replayed through an emulator), for `axelstra read` and flow reports
        "session.screen" => {
            let s = session()?;
            own_tree(sid()?)?;
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
        m => Err(unknown_method(m)),
    }
}

/// A conversation id read from `sid/<key>.json`, which the agent's own hook writes and so is untrusted: only a
/// UUID-shaped value (hex digits and dashes, 8 to 64 long) may become `meta.resume`, because the app puts it into a shell line.
fn valid_resume_id(id: &str) -> bool {
    (8..=64).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-')
}

/// The hook of a Claude session named a transcript file that is not on disk: the agent started but was never given a
/// prompt (Claude writes the file with the first one). A hook without `transcript_path` says nothing.
fn never_prompted(hook: &Value) -> bool {
    hook["transcript_path"].as_str().filter(|p| !p.is_empty()).is_some_and(|p| !std::path::Path::new(p).exists())
}

/// `session.main_no_tasks` when a spawn would put a task (an agent that is not the project main, or a worker) in a
/// project's main checkout: the project's own folder, not a worktree under `.axelstra-wt`. The folder is the spawn's
/// `meta.worktree`, else its `cwd`; a subfolder of the checkout counts as the checkout.
fn main_checkout_guard(reg: &Registry, p: &Value) -> Result<()> {
    let Some(dir) = p["meta"]["worktree"].as_str().filter(|w| !w.is_empty()).or_else(|| p["cwd"].as_str()) else { return Ok(()) };
    if axelstra_core::paths::norm(dir).contains("/.axelstra-wt/") {
        return Ok(());
    }
    let Some(project) = crate::projects::load(&reg.store).into_iter().find(|pr| axelstra_core::paths::is_within(dir, &pr.path)) else {
        return Ok(());
    };
    let has_main = {
        let sessions = reg.sessions.lock_recover();
        let metas = reg.metas.lock_recover();
        project_main_in(&sessions, &metas, &project.id).is_some()
    };
    if crate::notes::refused_in_main_checkout(&p["meta"], p["worker"] == true, has_main) {
        return Err(axelstra_core::coded!(
            "session.main_no_tasks",
            "the main checkout of {project} only takes its main session and terminals: give the task its own worktree with `axelstra dispatch new {project} <name> \"<request>\"`",
            project = project.name
        ));
    }
    Ok(())
}

/// How long one `session.spawn` with `queue: true` sleeps before it answers `queued` (the caller then comes back with its ticket).
const SPAWN_QUEUE_CHUNK: Duration = Duration::from_secs(20);

/// A refusal that goes away by itself (a worker finishes, memory recovers): the ones a queued spawn waits for.
fn is_queueable(e: &anyhow::Error) -> bool {
    axelstra_core::msg::find(e)
        .is_some_and(|c| matches!(c.code.as_str(), "session.worker_limit" | "governor.admission_denied" | "session.queued"))
}

/// A place in the spawn line, given up (and the next in line woken) when the `session.spawn` call ends, whichever way.
struct LinePlace {
    reg: Arc<Registry>,
    ticket: Option<u64>,
}

impl Drop for LinePlace {
    fn drop(&mut self) {
        if let Some(t) = self.ticket {
            if self.reg.spawnq.lock_recover().leave(t) {
                self.reg.pulse.bump();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{never_prompted, valid_resume_id};

    #[test]
    fn a_missing_transcript_means_no_request_was_ever_made() {
        let dir = std::env::temp_dir().join(format!("axelstra-never-prompted-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let there = dir.join("a.jsonl");
        std::fs::write(&there, "{}").unwrap();
        let hook = |p: &std::path::Path| serde_json::json!({"session_id": "abcdef12", "transcript_path": p.to_string_lossy()});
        assert!(never_prompted(&hook(&dir.join("missing.jsonl"))));
        assert!(!never_prompted(&hook(&there)));
        assert!(!never_prompted(&serde_json::json!({"session_id": "abcdef12"})), "no path, no verdict");
        assert!(!never_prompted(&serde_json::json!({"transcript_path": ""})));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_uuid_shaped_resume_ids_pass() {
        assert!(valid_resume_id("0f8fad5b-d9cb-469f-a165-70867728950e"));
        assert!(valid_resume_id("abcdef12"));
        for bad in ["", "abc", "x; calc", "$(id)", "0f8fad5b-d9cb-469f-a165-70867728950e; calc", "abc def12", "ghijklmn", &"a".repeat(65)] {
            assert!(!valid_resume_id(bad), "{bad}");
        }
    }
}
