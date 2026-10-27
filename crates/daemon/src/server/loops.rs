//! The background loops of the daemon: reapers, the scheduler, the janitor, the session monitor, the stall watchdog.
//! Each sleeps until an event wakes it (see `Pulse`); the timers are the moments something becomes due.

use super::*;

pub(super) static SWEEP_QUEUED: AtomicBool = AtomicBool::new(false);
/// A backoff retry of a pending removal is already waiting.
pub(super) static RETRY_QUEUED: AtomicBool = AtomicBool::new(false);

/// Kicks the janitor a few seconds from now (once for a burst of events): after a `done` or a merge, so the finished
/// worktree goes away without waiting for anything else. The delay lets the reaper close the session; it is one
/// delay per event, not a loop.
pub(super) fn sweep_soon(reg: &Arc<Registry>) {
    if SWEEP_QUEUED.swap(true, Ordering::SeqCst) {
        return;
    }
    let r = reg.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(4));
        SWEEP_QUEUED.store(false, Ordering::SeqCst);
        r.janitor.bump();
    });
}

/// Tells the main agent of the project that `item` (a sweep result with an `integrate.*` code) was left for a human or
/// for it. Typed into an agent, so it stays English.
pub(super) fn tell_main_not_integrated(reg: &Arc<Registry>, item: &Value) {
    let Some(project) = item["project"].as_str() else { return };
    let Some((id, s)) = reg.project_main(project).and_then(|id| reg.get(id).map(|s| (id, s))) else { return };
    let text = crate::prompts::not_integrated(
        item["path"].as_str().unwrap_or_default(),
        item["branch"].as_str().unwrap_or_default(),
        item["why"].as_str().unwrap_or_default(),
    );
    let m = reg.bus.send(None, id, "message", &text, None, None);
    reg.deliver_untrusted(id, s, &crate::msgbus::notice(&m, "axelstra"));
}

/// An agent that reported with `axelstra done` and then stays quiet for `REPORTED_IDLE_MS` is closed by itself (leads and
/// plain workers too; ephemeral ones have `spawn_reaper`): its process tree ends and its tab and record go. One the user
/// typed into after the report is theirs again and stays open. A session opened by hand that never said `done` is never
/// closed here. Woken by the pulse, one timer for the quiet spell.
pub(super) fn spawn_done_closer(reg: Arc<Registry>, id: u32) {
    std::thread::spawn(move || {
        let armed = Instant::now();
        let mut seen = reg.pulse.now();
        loop {
            let Some(s) = reg.get(id) else { return };
            if s.try_wait().is_some() || reg.bus.finished(id).is_none() {
                return;
            }
            if s.since_input_ms() < armed.elapsed().as_millis() as u64 {
                return;
            }
            let idle = s.idle_ms();
            if idle >= REPORTED_IDLE_MS {
                break;
            }
            seen = reg.pulse.wait(seen, Some(Duration::from_millis(REPORTED_IDLE_MS - idle + 20)));
        }
        let Some(s) = reg.get(id) else { return };
        // a question asked after the report is still a question
        if reg.wait_of(id, s.since_input_ms(), s.idle_ms()).is_some() {
            return;
        }
        let wt = reg.metas.lock_recover().get(&id).and_then(|m| m["worktree"].as_str().map(String::from));
        reg.tear_down(&[s]);
        reg.forget_session(id);
        if let Some(w) = wt {
            reg.note_activity(&w);
        }
        reg.persist();
        reg.pulse.bump();
        reg.janitor.bump();
    });
}

/// The shortest quiet spell after which an ephemeral worker may be nudged and then closed. An agent in a long tool call,
/// a slow network or a build that prints nothing is silent for a minute without being done; a plain command is not.
pub(crate) fn reaper_min_idle_secs(agent: bool) -> u64 {
    if agent {
        120
    } else {
        3
    }
}

pub(super) fn spawn_reaper(reg: Arc<Registry>, id: u32, parent: Option<u32>, idle_mode: bool, idle_secs: u64, lines: usize) {
    std::thread::spawn(move || {
        let mut worked = false;
        let mut done_by_idle = false;
        let mut nudged = false;
        let mut seen = reg.pulse.now();
        loop {
            let Some(s) = reg.get(id) else { return };
            let exited = s.try_wait();
            let idle = s.idle_ms();
            if idle < 3000 {
                worked = true;
            }
            if exited.is_some() || reg.bus.finished(id).is_some() {
                break;
            }
            if idle_mode && worked && idle >= idle_secs * 1000 {
                // it said it waits for an answer (hook event, ask, escalate): not finished, keep it. The parent was told
                // when the event came in.
                // or that it only waits for CI or a deploy (`axelstra await`): quiet on purpose
                if reg.wait_of(id, s.since_input_ms(), idle).is_some() || reg.awaiting_of(id, s.since_input_ms(), idle).is_some() {
                    worked = false;
                } else if !nudged && reg.metas.lock_recover().get(&id).map(|m| m["kind"] == "agent").unwrap_or(false) {
                    // first quiet spell: ask the worker to report through the bus instead of scraping its screen
                    crate::msgbus::deliver(s.clone(), crate::msgbus::paste(crate::prompts::IDLE_NUDGE));
                    nudged = true;
                    worked = false;
                } else {
                    done_by_idle = true;
                    break;
                }
            }
            // no polling: the program exiting, `axelstra done`, output after a quiet spell and every request bump the
            // pulse; the one timer is the moment the session would count as idle for `idle_secs`
            let limit = idle_secs * 1000;
            let timeout = (idle_mode && worked && idle < limit).then(|| Duration::from_millis(limit - idle + 20));
            seen = reg.pulse.wait(seen, timeout);
        }
        let Some(s) = reg.get(id) else { return };
        // give the output pipe a moment to drain after the process ended
        std::thread::sleep(Duration::from_millis(300));
        let code = s.try_wait();
        let done_msg = reg.bus.finished(id);
        // what the terminal shows, not the escape-code soup of a TUI redrawing itself
        // for an agent TUI only its last reply, not the banner, tool-run lines and status bar around it
        let tail = axelstra_core::screen::last_reply(&s.screen_lines(200)).unwrap_or_else(|| s.screen_lines(lines).join("\n"));
        let label = reg.labels.lock_recover().get(&id).cloned().unwrap_or_else(|| crate::prompts::session_name(id));
        let status = crate::prompts::worker_status(done_msg.is_some(), done_by_idle, code);
        if let Some(parent) = parent.and_then(|p| reg.get(p).map(|s| (p, s))) {
            let agent = reg.metas.lock_recover().get(&parent.0).and_then(|m| m["kind"].as_str().map(|k| k == "agent")).unwrap_or(false);
            let body = crate::prompts::worker_body(done_msg.as_ref().map(|d| d["text"].as_str().unwrap_or_default()), &tail);
            let msg = crate::prompts::worker_report(&label, &status, &body);
            // an agent TUI takes a multi-line message as one paste; a plain shell gets the single line
            let data = if agent {
                crate::msgbus::paste_with(&msg, !reg.runs_in_bypass(parent.0))
            } else {
                crate::prompts::worker_echo(&crate::msgbus::echo_safe(&label), &crate::msgbus::echo_safe(&status)).into_bytes()
            };
            crate::msgbus::deliver(parent.1.clone(), data);
        }
        s.kill();
        // the report went to the parent above; now the session leaves the registry (and the watchers hear it)
        reg.retire(id, code);
    });
}

/// Kills the live sessions whose worktree folder no longer exists on disk (they would only keep using memory).
/// Sessions without a worktree are left alone. No timer: called when something may have removed a worktree (the
/// project list is asked for, the watched worktree vanished, a janitor sweep or an orchestration ended).
pub(super) fn reap_orphans(reg: &Registry) {
    kill_sessions_where(reg, |w| !std::path::Path::new(w).exists());
}

/// Kills the live sessions whose worktree (from their meta) satisfies `pred`.
pub(super) fn kill_sessions_where(reg: &Registry, pred: impl Fn(&str) -> bool) {
    let doomed: Vec<_> = {
        let sessions = reg.sessions.lock_recover();
        let metas = reg.metas.lock_recover();
        sessions
            .iter()
            .filter(|(_, s)| s.try_wait().is_none())
            .filter(|(id, _)| metas.get(id).and_then(|m| m["worktree"].as_str()).is_some_and(&pred))
            .map(|(_, s)| s.clone())
            .collect()
    };
    for s in doomed {
        s.kill();
    }
}

/// Starts the automations when they are due. No tick: it sleeps until the next one is due, or until the list changes
/// (`auto_wake`). Missed runs are not replayed.
pub(super) fn spawn_scheduler(reg: Arc<Registry>) {
    tokio::spawn(async move {
        // folders parked by worktree removals in an earlier run
        let r = reg.clone();
        let _ = tokio::task::spawn_blocking(move || {
            for pr in crate::projects::load(&r.store) {
                for w in axelstra_core::git::worktree_list(std::path::Path::new(&pr.path)).unwrap_or_default() {
                    if let Some(parent) = std::path::Path::new(&w.path).parent() {
                        if !w.is_main {
                            axelstra_core::git::purge_trash(parent);
                        }
                    }
                }
            }
        })
        .await;
        loop {
            let r = reg.clone();
            let next = tokio::task::spawn_blocking(move || {
                let path = crate::automations::store_path(&r.store);
                // the file is claimed under the lock; the (slow) worktree and agent start run after it is released,
                // so a UI save is never stuck behind a running automation
                let claimed: Vec<_> = {
                    let _g = r.auto_lock.lock_recover();
                    let list = crate::automations::load(&path);
                    crate::automations::due_now(&list, crate::automations::now_ts())
                        .into_iter()
                        .filter_map(|id| match claim_automation(&path, &id) {
                            Ok(a) => Some(a),
                            Err(e) => {
                                eprintln!("automation {id} failed: {e:#}");
                                None
                            }
                        })
                        .collect()
                };
                for a in claimed {
                    if let Err(e) = launch_automation(&r, &a) {
                        eprintln!("automation {} failed: {e:#}", a.id);
                    }
                }
                let _g = r.auto_lock.lock_recover();
                crate::automations::next_due_ts(&crate::automations::load(&path), crate::automations::now_ts())
            })
            .await
            .ok()
            .flatten();
            match next {
                Some(at) => {
                    // a clock that jumped (hibernation) makes the wait wrong at worst by one wake-up
                    let wait = Duration::from_secs((at - crate::automations::now_ts()).clamp(1, 24 * 3600) as u64);
                    tokio::select! {
                        _ = tokio::time::sleep(wait) => {}
                        _ = reg.auto_wake.notified() => {}
                    }
                }
                None => reg.auto_wake.notified().await,
            }
        }
    });
}

/// Reads the spending and reports level crossings. With no budget set it sleeps until the settings change
/// (`budget_wake`); with one it looks every two minutes, as often as the agents' usage logs it reads are worth
/// reading (they have no event to wait on).
pub(super) fn spawn_budget_loop(reg: Arc<Registry>) {
    tokio::spawn(async move {
        loop {
            let r = reg.clone();
            let has = tokio::task::spawn_blocking(move || !misc::budget_check(&r).is_empty()).await.unwrap_or(false);
            if has {
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(120)) => {}
                    _ = reg.budget_wake.notified() => {}
                }
            } else {
                reg.budget_wake.notified().await;
            }
        }
    });
}

/// Tells the app when a file in the data directory that other processes write changes: `dispatch.json` is written by
/// the `axelstra dispatch` commands, which do not go through the daemon. Asleep until the OS reports a write.
pub(super) fn spawn_data_watch(reg: Arc<Registry>) {
    use notify::{RecursiveMode, Watcher};
    let Some(dir) = reg.saved.parent().map(|d| d.to_path_buf()) else { return };
    std::thread::spawn(move || {
        let (tx, rx) = std::sync::mpsc::channel();
        let Ok(mut w) = notify::recommended_watcher(tx) else { return };
        if w.watch(&dir, RecursiveMode::NonRecursive).is_err() {
            return;
        }
        while let Ok(ev) = rx.recv() {
            let hit = |ev: &notify::Result<notify::Event>| {
                ev.as_ref().is_ok_and(|e| {
                    // reading the file (inotify reports it) is not a change
                    !matches!(e.kind, notify::EventKind::Access(_))
                        && e.paths.iter().any(|p| p.file_name().is_some_and(|n| n == "dispatch.json"))
                })
            };
            if !hit(&ev) {
                continue;
            }
            // a write is several events (temp file, rename): one announcement for the burst
            std::thread::sleep(Duration::from_millis(150));
            while rx.try_recv().is_ok() {}
            reg.broadcast("dispatch.changed");
        }
    });
}

/// Removes the worktrees whose work is merged (see `janitor`) when something may have made one removable: at start,
/// when a session ends (its worktree is no longer busy), after a merge, a `done`, a finished flow or orchestration.
/// Kicks that come close together make one sweep, at most one a minute. Besides the events there is one timer: the
/// earliest moment a hand-made worktree stops counting as touched (`MANUAL_QUIET_SECS` after its last session left),
/// and a slow tick (30 min) for what raises no local event, like a PR merged on GitHub. `janitor.off` turns it off.
/// Wakes the janitor again after the backoff delay of attempt `attempts` (one waiting timer at a time).
pub(super) fn queue_janitor_retry(reg: &Arc<Registry>, attempts: u32) {
    if RETRY_QUEUED.swap(true, Ordering::SeqCst) {
        return;
    }
    let r = reg.clone();
    let delay = crate::janitor::retry_delay(attempts);
    std::thread::spawn(move || {
        std::thread::sleep(delay);
        RETRY_QUEUED.store(false, Ordering::SeqCst);
        r.janitor.bump();
    });
}

pub(super) fn spawn_janitor(reg: Arc<Registry>) {
    let gap = reg.timing.gap;
    std::thread::spawn(move || {
        let mut seen = 0;
        let mut last: Option<Instant> = None;
        let mut attempts = 0u32;
        loop {
            // events wake it; the timer is the next moment something becomes removable by time alone, or the slow tick
            seen = reg.janitor.wait(seen, Some(reg.next_janitor_wake()));
            if let Some(left) = last.map(|l| gap.saturating_sub(l.elapsed())).filter(|d| !d.is_zero()) {
                // the kicks of the next minute are folded into this sweep
                std::thread::sleep(left);
                seen = reg.janitor.now();
            }
            last = Some(Instant::now());
            if reg.closing.load(Ordering::SeqCst) {
                continue;
            }
            // worktrees of a removed project whose folder was held: the user asked for them to go, so this runs even
            // with the automatic cleanup off
            let (retried, held) = if crate::removal::has_pending(&reg.store) {
                crate::removal::retry(&reg.store, &|path| reg.close_worktree(path))
            } else {
                (0, 0)
            };
            if retried > 0 {
                reg.projects_changed();
            }
            if reg.saved.with_file_name("janitor.off").exists() {
                if held > 0 {
                    attempts += 1;
                    queue_janitor_retry(&reg, attempts);
                } else {
                    attempts = 0;
                }
                continue;
            }
            // a session waiting to be restored still needs its worktree
            let busy = |path: &str| reg.worktree_busy(path) || reg.worktree_pending(path);
            let close = |path: &str| reg.close_worktree(path);
            let touched = |path: &str| reg.worktree_touched(path);
            let env = crate::janitor::Env { busy: &busy, close: &close, touched: &touched };
            let swept = crate::janitor::sweep_with(&reg.store, &env, false);
            for s in swept.iter().chain(reg.clean_leftovers(false).iter()) {
                eprintln!("janitor: {s}");
                // finished work that could not be integrated is also reported to the project's main agent
                if s["code"].as_str().is_some_and(|c| c.starts_with("integrate.")) {
                    tell_main_not_integrated(&reg, s);
                }
            }
            if swept.iter().any(|s| s["removed"] == true) {
                reap_orphans(&reg);
                // the sessions saved for the removed worktrees go from sessions.json now
                reg.persist();
                reg.projects_changed();
            }
            // a folder nothing we watch holds was left whole: look again with a growing, bounded delay (see
            // `janitor::retry_delay`); a session that exits kicks the janitor at once, so no fixed timer runs
            if held > 0 || swept.iter().any(|s| s["pending"] == true) {
                attempts += 1;
                queue_janitor_retry(&reg, attempts);
            } else {
                attempts = 0;
            }
            reg.announce_events();
        }
    });
}

/// Tells watching connections (`session.changed`, see `ensure_session_notifier`) when the state of any session
/// changed: started, exited, went quiet or working again, escalated, got a message... Woken by those events (`pulse`);
/// its only timer is the moment the earliest working session would count as quiet, and with every session quiet it
/// sleeps until the next event. An agent that just went quiet or ended has its screen saved, so a daemon that is
/// killed or crashes still leaves the restored tab its history.
pub(super) fn spawn_session_monitor(reg: Arc<Registry>) {
    use axelstra_core::pty::QUIET_MS;
    std::thread::spawn(move || {
        let mut seen = reg.pulse.now();
        // (id, running, quiet, escalated, unread, finished, awaiting)
        let mut last: Vec<(u32, bool, bool, bool, usize, bool, bool)> = vec![];
        loop {
            let mut next: Option<u64> = None;
            let now: Vec<_> = {
                let sessions = reg.sessions.lock_recover();
                let mut ids: Vec<_> = sessions.keys().copied().collect();
                ids.sort();
                ids.into_iter()
                    .map(|id| {
                        let s = &sessions[&id];
                        let (running, idle) = (s.try_wait().is_none(), s.idle_ms());
                        if running && idle < QUIET_MS {
                            next = Some(next.map_or(QUIET_MS - idle, |n| n.min(QUIET_MS - idle)));
                        }
                        (
                            id,
                            running,
                            idle >= QUIET_MS,
                            reg.bus.is_escalated(id),
                            reg.bus.unread(id),
                            reg.bus.finished(id).is_some(),
                            reg.awaiting_of(id, s.view().since_input_ms(), idle).is_some(),
                        )
                    })
                    .collect()
            };
            if now != last {
                // working before, quiet or ended now
                let settled: Vec<u32> =
                    now.iter().filter(|n| (!n.1 || n.2) && last.iter().any(|l| l.0 == n.0 && l.1 && !l.2)).map(|n| n.0).collect();
                last = now;
                reg.broadcast("session.changed");
                save_histories_of(&reg, &settled);
            }
            seen = reg.pulse.wait(seen, next.map(|ms| Duration::from_millis(ms + 20)));
        }
    });
}

/// Fresh worktree + agent session with the automation's prompt. Records the start first so a failure never loops.
/// Marks the automation as started now and returns it. The caller holds `auto_lock`.
pub(super) fn claim_automation(path: &std::path::Path, id: &str) -> Result<crate::automations::Automation> {
    let mut list = crate::automations::load(path);
    let a = list
        .iter_mut()
        .find(|a| a.id == id)
        .ok_or_else(|| axelstra_core::coded!("automation.unknown", "unknown automation {id}", id = id))?;
    a.last_run = crate::automations::now_ts();
    let a = a.clone();
    crate::automations::save_all(path, &list)?;
    Ok(a)
}

pub(super) fn run_automation(reg: &Arc<Registry>, path: &std::path::Path, id: &str) -> Result<Value> {
    let a = {
        let _g = reg.auto_lock.lock_recover();
        claim_automation(path, id)?
    };
    launch_automation(reg, &a)
}

/// Fresh worktree + agent session for a claimed automation. Takes no lock of its own.
pub(super) fn launch_automation(reg: &Arc<Registry>, a: &crate::automations::Automation) -> Result<Value> {
    let pr = crate::projects::find(&reg.store, &a.project)?;
    let stamp = chrono::Local::now().format("%m%d-%H%M").to_string();
    let wt = crate::projects::worktree_create(&pr, &format!("auto-{}-{stamp}", a.name))?;
    // AXELSTRA_AGENT_CLI swaps the agent binary (a wrapper script, or a harmless stand-in in tests)
    let (program, args) = match std::env::var("AXELSTRA_AGENT_CLI") {
        Ok(cli) => crate::automations::raw_command(&cli, &a.prompt),
        Err(_) => {
            let ag = axelstra_core::agents::require(&a.agent)?;
            if ag.locate().is_none() {
                return Err(ag.missing_error());
            }
            crate::automations::agent_command(&a.agent, &a.prompt, axelstra_core::git::folder_trusted(std::path::Path::new(&wt.path)))?
        }
    };
    let spawn = Spawn { program, args, cwd: Some(wt.path.clone().into()), env: vec![], cols: 120, rows: 30 };
    let meta = json!({"project": pr.id, "worktree": wt.path, "kind": "agent", "automation": a.id});
    let sid = reg.spawn(&spawn, Some(&a.name), Some(meta))?;
    Ok(json!({"session": sid, "worktree": wt.path}))
}

/// Keeps a stray daemon from lingering: with no connection, no running session and no enabled automation for
/// ten minutes (`AXELSTRA_IDLE_EXIT_SECS`, 0 turns it off) it exits. The app normally stops it on quit.
/// No polling: it looks again only when something happened (`pulse`: a connection, a session exiting, a request),
/// and the one timer is the exit deadline, armed while idle.
pub(super) fn spawn_idle_exit(reg: Arc<Registry>) {
    let limit: u64 = std::env::var("AXELSTRA_IDLE_EXIT_SECS").ok().and_then(|v| v.parse().ok()).unwrap_or(600);
    if limit == 0 {
        return;
    }
    let limit = Duration::from_secs(limit);
    std::thread::spawn(move || {
        let mut seen = reg.pulse.now();
        let mut idle_since: Option<Instant> = None;
        loop {
            let busy = reg.clients.load(Ordering::SeqCst) > 0
                || reg.sessions.lock_recover().values().any(|s| s.try_wait().is_none())
                || crate::automations::load(&crate::automations::store_path(&reg.store)).iter().any(|a| a.enabled);
            if busy {
                idle_since = None;
                seen = reg.pulse.wait(seen, None);
                continue;
            }
            let left = limit.saturating_sub(idle_since.get_or_insert_with(Instant::now).elapsed());
            if left.is_zero() {
                reg.persist_now();
                std::process::exit(0);
            }
            seen = reg.pulse.wait(seen, Some(left));
        }
    });
}

pub(super) type Picked = (u32, Arc<Session>, axelstra_core::pty::View, Option<String>, Option<Value>);

/// Parents of the sessions that still run for them: a parent waiting on live children is not stalled.
pub(super) fn busy_parents(reg: &Registry, picked: &[Picked]) -> std::collections::HashSet<u32> {
    picked
        .iter()
        .filter(|(id, s, ..)| s.try_wait().is_none() && reg.bus.finished(*id).is_none())
        .filter_map(|(_, _, _, _, m)| m.as_ref().and_then(|m| m["parent"].as_u64()).map(|p| p as u32))
        .collect()
}

/// What the stall watchdog needs to know about a session (see `stall::is_stalled`).
pub(super) fn stall_facts(
    reg: &Registry,
    id: u32,
    running: bool,
    idle_ms: u64,
    since_input_ms: u64,
    meta: Option<&Value>,
    busy: &std::collections::HashSet<u32>,
) -> crate::stall::Facts {
    crate::stall::Facts {
        agent: meta.is_some_and(|m| m["kind"] == "agent"),
        main: meta.is_some_and(|m| m["main"] == true),
        running,
        reported: reg.bus.finished(id).is_some(),
        escalated: reg.bus.is_escalated(id),
        awaiting: reg.awaiting_of(id, since_input_ms, idle_ms).is_some(),
        live_children: busy.contains(&id),
        idle_ms,
        since_input_ms,
    }
}

/// Every 10 s: an agent that became stalled gets one message to its parent (until it works again).
pub(super) fn spawn_stall_watchdog(reg: Arc<Registry>) {
    std::thread::spawn(move || loop {
        // tests shorten the threshold to seconds and then need a faster pass too
        std::thread::sleep(if std::env::var_os("AXELSTRA_STALL_SECS").is_some() {
            Duration::from_millis(300)
        } else {
            Duration::from_secs(10)
        });
        stall_tick(&reg);
    });
}

/// One pass of the watchdog.
pub(super) fn stall_tick(reg: &Arc<Registry>) {
    let threshold = reg.with_settings(crate::stall::threshold_ms);
    let minutes = crate::stall::display_minutes(threshold);
    let picked: Vec<Picked> = {
        let l = reg.sessions.lock_recover();
        let labels = reg.labels.lock_recover();
        let metas = reg.metas.lock_recover();
        l.iter().map(|(id, s)| (*id, s.clone(), s.view(), labels.get(id).cloned(), metas.get(id).cloned())).collect()
    };
    let busy = busy_parents(reg, &picked);
    let mut stalled = Vec::new();
    for (id, s, view, label, meta) in &picked {
        let facts = stall_facts(reg, *id, s.try_wait().is_none(), view.idle_ms(), view.since_input_ms(), meta.as_ref(), &busy);
        if crate::stall::is_stalled(&facts, threshold) {
            let parent = meta.as_ref().and_then(|m| m["parent"].as_u64()).map(|p| p as u32);
            stalled.push((*id, label.clone(), parent, view.screen_tail(12)));
        }
    }
    let ids: std::collections::HashSet<u32> = stalled.iter().map(|x| x.0).collect();
    let fresh: Vec<_> = {
        let mut seen = reg.stall_notified.lock_recover();
        seen.retain(|id| ids.contains(id));
        stalled.into_iter().filter(|x| seen.insert(x.0)).collect()
    };
    for (id, label, parent, screen) in fresh {
        let Some((pid, ps)) = parent.and_then(|p| reg.get(p).map(|s| (p, s))) else { continue };
        let label = label.unwrap_or_else(|| format!("session {id}"));
        let text = crate::stall::parent_text(&label, id, minutes, &crate::stall::last_line(&screen));
        let m = reg.bus.send(Some(id), pid, "message", &text, None, None);
        let agent = reg.metas.lock_recover().get(&pid).map(|m| m["kind"] == "agent").unwrap_or(false);
        if agent {
            reg.deliver_untrusted(pid, ps, &crate::msgbus::notice(&m, &format!("{label} ({})", crate::prompts::session_name(id))));
        }
    }
}
