//! Hibernation of agent sessions: `session.hibernate`, `session.wake`, the wake queue, the policy deadline and the list
//! rows. The pure parts (setting, deadline, wake command, bookkeeping) are in `crate::hibernate`; see AGENTS.md,
//! section "Hibernation". Nothing here polls: the policy thread sleeps until the earliest due session or a pulse, the
//! waker until a pulse, a settling session until the moment its output has been quiet long enough.

use super::*;
use crate::hibernate as hib;
use axelstra_core::msg::Coded;

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

/// The watcher of a process that ended asks whether that was a hibernation (or the session already woke under the same
/// id), in which case the session did not end.
pub(super) fn superseded(reg: &Registry, id: u32, s: &Arc<Session>) -> bool {
    match reg.get(id) {
        Some(cur) => !Arc::ptr_eq(&cur, s),
        None => {
            let was = reg.hib.is_hibernating(id);
            if was {
                reg.hib.end_hibernating(id);
                // a wake waiting for the old process to be seen as gone may go on
                reg.pulse.bump();
            }
            was || reg.hib.is_sleeping(id)
        }
    }
}

fn mb(bytes: u64) -> u64 {
    bytes / (1024 * 1024)
}

/// The conversation id to resume: the one the agent's hook wrote last (a `/clear` replaces it), else the one the tab was
/// opened with, else its key. For claude it must be a conversation with replies that exists on disk; codex and gemini
/// resume the folder's latest conversation and need none.
fn resume_id(reg: &Registry, meta: &Value, spec: &Value) -> Option<String> {
    if meta["agent"].as_str().is_some_and(|a| a != "claude") {
        return None;
    }
    let key = meta["key"].as_str().filter(|k| !k.is_empty() && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'));
    let sid_file = key.map(|k| reg.saved.with_file_name("sid").join(format!("{k}.json")));
    let latest = sid_file
        .and_then(|f| std::fs::read_to_string(f).ok())
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|j| j["session_id"].as_str().map(String::from));
    let cwd = spec["cwd"].as_str().or(meta["worktree"].as_str()).unwrap_or_default();
    [latest, meta["resume"].as_str().map(String::from), key.map(String::from)]
        .into_iter()
        .flatten()
        .filter(|id| hib::valid_resume_id(id))
        .find(|id| crate::agentlog::claude_usage(cwd, Some(id)).is_some_and(|u| u.messages > 0))
}

struct Plan {
    s: Arc<Session>,
    resume: Option<String>,
    mem: u64,
}

/// Every reason a session must not be put to sleep, as a coded error. Reads only; IO happens outside the registry locks.
fn check(reg: &Arc<Registry>, id: u32) -> Result<Plan> {
    let s = reg
        .get(id)
        .filter(|s| s.try_wait().is_none())
        .ok_or_else(|| axelstra_core::coded!("session.not_found", "no such session {id}", id = id))?;
    let (meta, spec) = (reg.metas.lock_recover().get(&id).cloned().unwrap_or(Value::Null), reg.specs.lock_recover().get(&id).cloned());
    if meta["kind"] != "agent" {
        return Err(axelstra_core::coded!("hibernate.not_agent", "only agent sessions can be hibernated"));
    }
    if meta["main"] == true {
        return Err(axelstra_core::coded!("hibernate.main", "the project's main session is never hibernated"));
    }
    let agent = meta["agent"].as_str().unwrap_or_default().to_string();
    let no_resume = || {
        axelstra_core::coded!(
            "hibernate.no_resume",
            "{agent} cannot resume this conversation: no support for it or no saved conversation yet",
            agent = agent.clone()
        )
    };
    if !axelstra_core::agents::get(&agent).is_some_and(|a| a.resume.is_some()) {
        return Err(no_resume());
    }
    let (idle, input) = (s.idle_ms(), s.since_input_ms());
    if input < hib::quiet_ms() || reg.hib.events(id).1 < hib::quiet_ms() {
        return Err(axelstra_core::coded!("hibernate.recent", "it got input or news in the last seconds"));
    }
    if idle < hib::quiet_ms()
        || reg
            .hints
            .state_of(id)
            .is_some_and(|(st, age)| st == axelstra_core::waiting::HintState::Working && age < axelstra_core::waiting::HINT_TTL_MS)
    {
        return Err(axelstra_core::coded!("hibernate.working", "the agent is working"));
    }
    // a frozen tree (or a frozen build in it) is the governor's: it ends only by `mem.resume`, never by sleeping
    if reg.pauser.paused_of(id).is_some() {
        return Err(axelstra_core::coded!("hibernate.paused", "the session is paused by the memory governor: resume it first"));
    }
    if reg.wait_of(id, input, idle).is_some() {
        return Err(axelstra_core::coded!("hibernate.waiting", "the agent waits for an answer or a permission"));
    }
    let spec = spec.ok_or_else(no_resume)?;
    // codex and gemini continue "the latest conversation of the folder": with another session of the same agent in it,
    // that could be the wrong one
    if agent != "claude" {
        let wt = axelstra_core::paths::norm(meta["worktree"].as_str().unwrap_or_default());
        let clash = reg.metas.lock_recover().iter().any(|(i, m)| {
            *i != id
                && m["kind"] == "agent"
                && m["agent"].as_str() == Some(agent.as_str())
                && axelstra_core::paths::norm(m["worktree"].as_str().unwrap_or_default()) == wt
        });
        if clash {
            return Err(axelstra_core::coded!(
                "hibernate.ambiguous",
                "another {agent} session shares this folder: resuming could pick the wrong conversation",
                agent = agent.clone()
            ));
        }
    }
    let resume = resume_id(reg, &meta, &spec);
    if hib::wake_command(&spec, &meta, resume.as_deref()).is_err() {
        return Err(no_resume());
    }
    // a command in the foreground, a build, a lock held or a hook in flight is a process below the agent's shell
    if let Some(pid) = s.pid() {
        let snap = axelstra_core::procs::snapshot();
        let mut below = snap.descendants(&[pid]);
        below.remove(&pid);
        if axelstra_core::procs::names_of(&below).values().any(|n| hib::busy_process(n)) {
            return Err(axelstra_core::coded!("hibernate.command", "a command, a build or a hook is running in the session"));
        }
    }
    let mem = s.memory_now();
    Ok(Plan { s, resume, mem })
}

/// Ends the process tree of `id` and keeps what is needed to bring it back. Blocking (the tree is killed and waited for).
pub(super) fn hibernate(reg: &Arc<Registry>, id: u32) -> Result<Value> {
    if let Some(sl) = reg.hib.sleeper(id) {
        return Ok(json!({"id": id, "hibernated": {"since": sl.since, "saved_mb": mb(sl.saved_bytes)}}));
    }
    let seen = reg.hib.events(id).0;
    let plan = check(reg, id)?;
    save_histories_of(reg, &[id]);
    // the commit: the session leaves `sessions` and enters the sleepers in one step, and only if no event arrived since
    // `seen` (a message, a done, a wake, input): that event wins, the session stays as it is
    {
        let mut sessions = reg.sessions.lock_recover();
        let same = sessions.get(&id).is_some_and(|x| Arc::ptr_eq(x, &plan.s));
        if !same || !reg.hib.commit(id, seen, plan.mem) {
            return Err(axelstra_core::coded!("hibernate.event", "something reached the session meanwhile: it stays awake"));
        }
        sessions.remove(&id);
    }
    let since = reg.hib.sleeper(id).map_or_else(now_ms, |s| s.since);
    // the spec keeps the role and the conversation, so a daemon restart brings the row back as hibernated
    {
        let meta = reg.metas.lock_recover().get(&id).cloned().unwrap_or(Value::Null);
        let mut specs = reg.specs.lock_recover();
        if let Some(sp) = specs.get_mut(&id).filter(|sp| sp["meta"].is_object()) {
            for k in ["lead", "worker", "agent", "key", "project", "worktree", "kind"] {
                if !meta[k].is_null() {
                    sp["meta"][k] = meta[k].clone();
                }
            }
            if let Some(r) = &plan.resume {
                sp["meta"]["resume"] = json!(r);
            }
            sp["meta"]["hibernated"] = json!({"since": since, "saved_mb": mb(plan.mem)});
        }
    }
    reg.persist();
    // paused after the check: continue the tree first, a stopped process would not leave the table cleanly
    reg.resume_before_kill(id);
    reg.tear_down(&[plan.s]);
    reg.hints.clear(id);
    reg.modes.lock_recover().remove(&id);
    reg.awaits.clear(id);
    reg.broadcast("session.changed");
    reg.broadcast_event(json!({"event": "session.hibernated", "id": id}));
    reg.pulse.bump();
    reg.janitor.bump();
    Ok(json!({"id": id, "hibernated": {"since": since, "saved_mb": mb(plan.mem)}}))
}

/// Something concerned `id`: if it sleeps, it wakes (queued behind the ones that asked first when no slot is free).
/// Always counts as an event, so a hibernation in progress gives up. True when a wake was started or queued.
pub(super) fn wake(reg: &Arc<Registry>, id: u32) -> bool {
    reg.hib.touch(id);
    let queued = reg.hib.enqueue(id);
    if queued {
        reg.pulse.bump();
    }
    queued
}

/// Wakes `id` and waits (at most `timeout`) until its process exists, or, with `settled`, until its inbox notice was
/// typed too. False: it did not come back in time, or never will (it is gone, or the resume failed).
pub(super) fn wake_wait(reg: &Arc<Registry>, id: u32, settled: bool, timeout: Duration) -> bool {
    wake(reg, id);
    let end = Instant::now() + timeout;
    loop {
        let seen = reg.pulse.now();
        let sleeping = reg.hib.is_sleeping(id);
        let waking = reg.hib.is_waking(id).is_some();
        if reg.get(id).is_some() && !sleeping && !(settled && waking) {
            return true;
        }
        if !sleeping && !waking && reg.get(id).is_none() {
            return false;
        }
        let now = Instant::now();
        if now >= end {
            return false;
        }
        reg.pulse.wait(seen, Some(end - now));
    }
}

/// A worker needs a free slot to come back; a lead or a plain agent does not count against the limit.
fn admit(reg: &Arc<Registry>, id: u32) -> bool {
    if reg.metas.lock_recover().get(&id).is_none_or(|m| m["worker"] != true) {
        return true;
    }
    let alive = |i: u32| reg.get(i).map(|s| s.try_wait().is_none()).unwrap_or(false);
    reg.bus.live_workers(alive) < reg.bus.limit()
}

/// The one thread that brings sessions back, in the order they asked. It sleeps until a pulse (a wake asked for, a
/// session ended and freed a slot, the limit changed): never on a timer.
pub(super) fn spawn_waker(reg: Arc<Registry>) {
    std::thread::spawn(move || loop {
        let seen = reg.pulse.now();
        match reg.hib.queue().into_iter().find(|id| admit(&reg, *id)) {
            Some(id) => start_wake(&reg, id),
            None => {
                reg.pulse.wait(seen, None);
            }
        }
    });
}

fn start_wake(reg: &Arc<Registry>, id: u32) {
    // the watcher of the old process must have seen it end, or it could take the new process for it
    let end = Instant::now() + Duration::from_secs(5);
    while reg.hib.is_hibernating(id) && Instant::now() < end {
        let seen = reg.pulse.now();
        reg.pulse.wait(seen, Some(end.saturating_duration_since(Instant::now())));
    }
    reg.hib.end_hibernating(id);
    if reg.hib.begin_wake(id).is_none() {
        return;
    }
    let (meta, spec) = (
        reg.metas.lock_recover().get(&id).cloned().unwrap_or(Value::Null),
        reg.specs.lock_recover().get(&id).cloned().unwrap_or(Value::Null),
    );
    let resume = resume_id(reg, &meta, &spec);
    let spawned = (|| -> Result<()> {
        let w = hib::wake_command(&spec, &meta, resume.as_deref()).map_err(|_| {
            axelstra_core::coded!(
                "hibernate.no_resume",
                "{agent} cannot resume this conversation: no support for it or no saved conversation yet",
                agent = meta["agent"].as_str().unwrap_or_default().to_string()
            )
        })?;
        if let Some(cwd) = w.cwd.as_deref().filter(|c| !std::path::Path::new(c).is_dir()) {
            return Err(axelstra_core::coded!(
                "session.cwd_missing",
                "folder {path} does not exist: {program} was not started",
                path = cwd.to_string(),
                program = w.program.clone()
            ));
        }
        let sp = Spawn { program: w.program, args: w.args, cwd: w.cwd.map(Into::into), env: w.env, cols: w.cols, rows: w.rows };
        reg.spawn_as(id, &sp, None, None)?;
        Ok(())
    })();
    if let Err(e) = spawned {
        let detail = format!("{e:#}");
        fail_wake(reg, id, None, &detail);
        return;
    }
    // the saved terminal shows dimmed above the new output; a worker counts against the limit again; its reaper is armed again
    if let (Some(key), Some(sess)) = (meta["key"].as_str(), reg.get(id)) {
        if let Some(text) = take_history(reg, key) {
            sess.seed(text.as_bytes());
        }
    }
    if meta["worker"] == true {
        reg.bus.add_worker(id);
    }
    if let Some(r) = reg.hib.reap_of(id) {
        let parent = r.get("parent").and_then(Value::as_u64).map(|x| x as u32);
        let idle_mode = r.get("mode").and_then(Value::as_str) == Some("idle");
        let idle_secs = r.get("idle_secs").and_then(Value::as_u64).unwrap_or(20).max(reaper_min_idle_secs(true));
        let lines = r.get("lines").and_then(Value::as_u64).unwrap_or(40).clamp(1, 400) as usize;
        spawn_reaper(reg.clone(), id, parent, idle_mode, idle_secs, lines);
    }
    {
        let mut specs = reg.specs.lock_recover();
        if let Some(sp) = specs.get_mut(&id).filter(|sp| sp["meta"].is_object()) {
            sp["meta"].as_object_mut().map(|o| o.remove("hibernated"));
        }
    }
    reg.persist();
    reg.broadcast("session.changed");
    reg.pulse.bump();
    let r = reg.clone();
    std::thread::spawn(move || settle(r, id));
}

/// Waits (a computed deadline, no polling) until the woken agent has printed its banner and gone quiet, then types the
/// notices of what reached it while it slept. Only then is it "working" again. If it exits before that, the resume failed.
fn settle(reg: Arc<Registry>, id: u32) {
    let begun = Instant::now();
    let min_total = Duration::from_secs(4);
    loop {
        let seen = reg.pulse.now();
        let Some(s) = reg.get(id) else {
            fail_wake(&reg, id, None, "the process ended");
            return;
        };
        if let Some(code) = s.try_wait() {
            let why = if code == hib::RESUME_FAILED_EXIT {
                "the agent could not resume the conversation".to_string()
            } else {
                format!("the process ended with code {code}")
            };
            fail_wake(&reg, id, Some(code), &why);
            return;
        }
        let up = begun.elapsed();
        if up >= hib::SETTLE_CAP || (s.idle_ms() >= hib::SETTLE_QUIET_MS && up >= min_total) {
            break;
        }
        let quiet_left = hib::SETTLE_QUIET_MS.saturating_sub(s.idle_ms());
        let wait = Duration::from_millis(quiet_left.max(1) + 20).max(min_total.saturating_sub(up)).min(hib::SETTLE_CAP.saturating_sub(up));
        reg.pulse.wait(seen, Some(wait));
    }
    if let Some(s) = reg.get(id) {
        let unread = reg.bus.inbox(id, false, false);
        if !unread.is_empty() {
            let text = unread
                .iter()
                .map(|m| {
                    let from = m
                        .from
                        .map(|f| reg.labels.lock_recover().get(&f).cloned().unwrap_or_else(|| crate::prompts::session_name(f)))
                        .unwrap_or_else(|| crate::prompts::USER_SENDER.into());
                    crate::msgbus::notice(m, &from)
                })
                .collect::<Vec<_>>()
                .join("\n");
            reg.deliver_untrusted(id, s, &text);
        }
    }
    if !reg.hib.finish_waking(id) {
        // the process ended meanwhile and its failure was handled
        return;
    }
    reg.broadcast("session.changed");
    reg.broadcast_event(json!({"event": "session.woke", "id": id}));
    reg.pulse.bump();
}

/// The resume failed: the session becomes `exited` with a coded error (it never stays hibernated), and its parent is told.
/// Both the settle thread and the exit watcher of the new process can see the failure first (a resume that fails exits
/// at once); whoever ends the wake handles it, the other does nothing, and `wake_error` is stored before the retire.
pub(super) fn fail_wake(reg: &Arc<Registry>, id: u32, code: Option<i32>, detail: &str) {
    if !reg.hib.finish_waking(id) {
        return;
    }
    let label = reg.labels.lock_recover().get(&id).cloned().unwrap_or_else(|| crate::prompts::session_name(id));
    let c = Coded::new(
        "hibernate.wake_failed",
        format!("{label} could not be brought back after hibernation: {detail}"),
        &[("id", id.to_string()), ("label", label.clone()), ("detail", detail.to_string())],
    );
    let err = json!({"code": c.code, "params": c.params_json(), "text": c.to_string()});
    let parent = reg.metas.lock_recover().get(&id).and_then(|m| m["parent"].as_u64()).map(|p| p as u32);
    reg.hib.set_failed(id, err.clone());
    if let Some(s) = reg.get(id) {
        reg.tear_down(&[s]);
    }
    if !reg.retire(id, code.or(Some(hib::RESUME_FAILED_EXIT))) {
        reg.forget_session(id);
        reg.persist();
    }
    reg.broadcast_event(
        json!({"event": "session.wake_failed", "id": id, "code": c.code, "params": c.params_json(), "error": c.to_string()}),
    );
    if let Some(p) = parent {
        let text = crate::prompts::wake_failed_report(&label, &c.to_string());
        if wake_if_sleeping(reg, p, &text) {
            return;
        }
        let m = reg.bus.send(Some(id), p, "message", &text, None, None);
        if let Some(ps) = reg.get(p) {
            reg.deliver_untrusted(p, ps, &crate::msgbus::notice(&m, &label));
        }
    }
}

/// Leaves `text` in the inbox of a hibernated session and wakes it (it is typed once it is back). False: it is awake.
fn wake_if_sleeping(reg: &Arc<Registry>, id: u32, text: &str) -> bool {
    if !reg.hib.is_sleeping(id) {
        return false;
    }
    reg.bus.send(None, id, "message", text, None, None);
    wake(reg, id);
    true
}

/// Brings the sessions that were hibernated when the previous daemon stopped back as hibernated rows (new ids; the
/// app must not restore them). Their inbox did not survive the restart, like everything the bus holds.
pub(super) fn adopt(reg: &Arc<Registry>) {
    let taken: Vec<Value> = {
        let mut pending = reg.pending.lock_recover();
        let (sleepers, rest): (Vec<Value>, Vec<Value>) = pending.drain(..).partition(|r| r["meta"]["hibernated"].is_object());
        *pending = rest;
        sleepers
    };
    if taken.is_empty() {
        return;
    }
    for rec in taken {
        let id = reg.next.fetch_add(1, Ordering::SeqCst) + 1;
        let h = rec["meta"]["hibernated"].clone();
        let mut meta = rec["meta"].clone();
        if let Some(o) = meta.as_object_mut() {
            o.remove("hibernated");
            // the parent's id belonged to the previous run
            o.remove("parent");
        }
        if let Some(l) = rec["label"].as_str() {
            reg.labels.lock_recover().insert(id, l.to_string());
        }
        reg.metas.lock_recover().insert(id, meta);
        reg.specs.lock_recover().insert(id, rec);
        reg.hib.adopt(id, h["since"].as_u64().unwrap_or_else(now_ms), h["saved_mb"].as_u64().unwrap_or(0) * 1024 * 1024);
    }
    reg.persist();
}

/// Adds the `hibernated` field to every row of `session.list` and the rows of the sleeping sessions.
pub(super) fn decorate(reg: &Registry, rows: &mut Vec<Value>) {
    for r in rows.iter_mut() {
        let id = r["id"].as_u64().unwrap_or(0) as u32;
        r["hibernated"] = match reg.hib.is_waking(id) {
            Some(since) => json!({"since": since, "saved_mb": null, "waking": true, "queued": false}),
            None => Value::Null,
        };
    }
    // a session between its sleep and its process (or just woken, process not yet in the list) keeps a row, so a
    // frontend never sees it vanish
    let known: std::collections::HashSet<u64> = rows.iter().filter_map(|r| r["id"].as_u64()).collect();
    let mut sleepers: Vec<(u32, hib::Sleeper, bool)> = reg.hib.sleepers().into_iter().map(|(i, s)| (i, s, false)).collect();
    for (id, since) in reg.hib.wakings() {
        sleepers.push((id, hib::Sleeper { since, saved_bytes: 0 }, true));
    }
    sleepers.retain(|(id, _, _)| !known.contains(&u64::from(*id)));
    if sleepers.is_empty() {
        return;
    }
    for (id, sl, waking) in sleepers {
        let (label, meta) = (reg.labels.lock_recover().get(&id).cloned(), reg.metas.lock_recover().get(&id).cloned());
        // `running` stays true: the session did not end, and a frontend that reads false as "exited" must not close its tab
        rows.push(json!({"id": id, "running": true, "idle_ms": now_ms().saturating_sub(sl.since), "waiting": false, "wait": null,
            "awaiting": null, "stalled": false, "stall": null, "escalated": reg.bus.is_escalated(id), "unread": reg.bus.unread(id),
            "finished": reg.bus.finished(id).is_some(), "label": label, "meta": meta,
            "hibernated": {"since": sl.since, "saved_mb": mb(sl.saved_bytes), "waking": waking, "queued": reg.hib.queued(id)}}));
    }
    rows.sort_by_key(|x| x["id"].as_u64());
}

fn has_live_child(reg: &Registry, id: u32) -> bool {
    let sessions = reg.sessions.lock_recover();
    let metas = reg.metas.lock_recover();
    metas.iter().any(|(c, m)| {
        m["parent"].as_u64() == Some(u64::from(id))
            && sessions.get(c).is_some_and(|s| s.try_wait().is_none())
            && reg.bus.finished(*c).is_none()
    })
}

/// The policy thread: one deadline per session (the moment it will have waited for the configured time), one wake-up
/// for the earliest. It looks again on a pulse (state changes are events) and when a refused session's backoff ends.
pub(super) fn spawn_policy(reg: Arc<Registry>) {
    std::thread::spawn(move || {
        let _site = axelstra_core::proc::site("hibernate_policy");
        let mut seen = reg.pulse.now();
        loop {
            let next = policy_tick(&reg);
            seen = reg.pulse.wait(seen, next);
        }
    });
}

pub(super) fn policy_tick(reg: &Arc<Registry>) -> Option<Duration> {
    let p = reg.with_settings(hib::policy);
    if !p.enabled {
        return None;
    }
    let picked: Vec<(u32, Arc<Session>, Value)> = {
        let sessions = reg.sessions.lock_recover();
        let metas = reg.metas.lock_recover();
        sessions
            .iter()
            .filter_map(|(id, s)| metas.get(id).filter(|m| m["kind"] == "agent" && m["main"] != true).map(|m| (*id, s.clone(), m.clone())))
            .collect()
    };
    let mut next: Option<Duration> = None;
    let mut soonest = |d: Duration| next = Some(next.map_or(d, |n| n.min(d)));
    for (id, s, meta) in picked {
        if s.try_wait().is_some() || reg.hib.is_waking(id).is_some() {
            continue;
        }
        let (idle, input) = (s.idle_ms(), s.since_input_ms());
        let facts = hib::Facts {
            idle_ms: idle,
            since_input_ms: input,
            since_event_ms: reg.hib.events(id).1,
            awaiting: reg.awaiting_of(id, input, idle).is_some(),
            finished: reg.bus.finished(id).is_some(),
            lead_with_children: meta["lead"] == true && has_live_child(reg, id),
            main: false,
        };
        let Some(due) = hib::due_in(&facts, &p) else { continue };
        if let Some(left) = reg.hib.backoff_left(id) {
            soonest(left);
            continue;
        }
        if !due.is_zero() {
            soonest(due + Duration::from_millis(20));
            continue;
        }
        if hibernate(reg, id).is_err() {
            soonest(reg.hib.backoff(id));
        }
    }
    next
}

pub(super) fn call(c: &mut Ctx) -> Result<Value> {
    let (p, reg) = (c.p, c.reg);
    let id = p.get("id").and_then(Value::as_u64).and_then(|x| u32::try_from(x).ok()).ok_or_else(|| missing("id"))?;
    let known = reg.get(id).is_some() || reg.hib.is_sleeping(id) || reg.hib.is_waking(id).is_some();
    if !known {
        return Err(axelstra_core::coded!("session.not_found", "no such session {id}", id = id));
    }
    match c.req.method.as_str() {
        "session.hibernate" => tokio::task::block_in_place(|| hibernate(reg, id)),
        _ => {
            let state = |reg: &Arc<Registry>| {
                if reg.hib.is_waking(id).is_some() || (reg.get(id).is_some() && !reg.hib.is_sleeping(id)) {
                    if reg.hib.is_waking(id).is_some() {
                        "waking"
                    } else {
                        "awake"
                    }
                } else if reg.hib.queued(id) {
                    "queued"
                } else {
                    "waking"
                }
            };
            if p["wait"].as_bool() == Some(true) {
                let ok = tokio::task::block_in_place(|| wake_wait(reg, id, true, Duration::from_secs(60)));
                return Ok(json!({"id": id, "state": if ok { "awake" } else { state(reg) }}));
            }
            wake(reg, id);
            Ok(json!({"id": id, "state": state(reg)}))
        }
    }
}
