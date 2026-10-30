use super::*;

pub(super) fn call(c: &mut Ctx) -> Result<Value> {
    let req = c.req;
    let (p, reg) = (c.p, c.reg);
    let agent = &*c.agent;
    match req.method.as_str() {
        "msg.send" => {
            let to = p["to"].as_u64().and_then(|x| u32::try_from(x).ok()).ok_or_else(|| missing("to"))?;
            // an agent speaks as itself: the sender is the session its connection authenticated as, whatever `from` says
            let from = match *agent {
                Some(me) => Some(me),
                None => p["from"].as_u64().and_then(|x| u32::try_from(x).ok()),
            };
            let kind = p["kind"].as_str().unwrap_or("message");
            if !["message", "ask", "reply", "done", "escalate"].contains(&kind) {
                return Err(axelstra_core::coded!("msg.unknown_kind", "unknown message kind {kind}", kind = kind));
            }
            // news for a hibernated session wakes it (and stops a hibernation under way): the message waits in its inbox
            // and is typed once the agent is back
            reg.hib.touch(to);
            let sleeping = reg.hib.is_sleeping(to);
            let target = if sleeping {
                None
            } else {
                Some(reg.get(to).ok_or_else(|| axelstra_core::coded!("session.not_found", "no such session {id}", id = to))?)
            };
            let text = p["text"].as_str().unwrap_or_default();
            // the same report or escalation again, with nothing in between: stored, but no second notice
            let repeat = reg.bus.repeats(from, kind, text);
            let m = reg.bus.send(from, to, kind, text, p["reply_to"].as_u64(), p["thread"].as_str().map(String::from));
            if sleeping {
                hibernate::wake(reg, to);
            }
            // `axelstra ask` and `axelstra wait` sleep until the watch stream speaks. The session monitor only speaks when
            // its summary of a session moved (unread count, escalated, finished...), and a reply that lands in the same
            // tick as another change of that summary (the asker read its inbox meanwhile) leaves it as it was: the
            // message announces itself, so no answer waits for an unrelated change.
            reg.broadcast("session.changed");
            // a finished agent is not restored; new work for it (a message or reply) makes it restorable again
            if let (true, Some(f)) = (kind == "done", from) {
                reg.set_done(f, true);
            } else if kind == "message" || kind == "reply" {
                reg.set_done(to, false);
            }
            // the summary of a finished agent goes into the Relatorio section of its task file
            if let (true, Some(f)) = (kind == "done", from) {
                if let Some(note) = note_path(reg, f) {
                    let (status, body) = crate::notes::split_status(text);
                    let _ = crate::notes::append(&note, status, body);
                }
                // a lead or `--report-to` worker finished: its worktree is integrated by the next sweep
                let lead = reg.metas.lock_recover().get(&f).is_some_and(|m| m["lead"] == true);
                if lead || reg.bus.is_reaped(f) {
                    let wt = reg.metas.lock_recover().get(&f).and_then(|m| m["worktree"].as_str().map(String::from));
                    if let Some(wt) = wt {
                        crate::integrate::mark_done(&wt, crate::notes::split_status(text).0 == "ok");
                    }
                }
                // an agent left running after its report is closed once it is quiet; ephemeral ones have their reaper
                if let Some(f) =
                    from.filter(|f| !reg.bus.is_reaped(*f) && reg.metas.lock_recover().get(f).is_some_and(|m| m["kind"] == "agent"))
                {
                    spawn_done_closer(reg.clone(), f);
                }
                sweep_soon(reg);
            }
            // agents get a one-line notice; a worker with a reaper reports through it instead
            let quiet =
                repeat || p["quiet"].as_bool() == Some(true) || (kind == "done" && from.map(|f| reg.bus.is_reaped(f)).unwrap_or(false));
            let agent = reg.metas.lock_recover().get(&to).map(|m| m["kind"] == "agent").unwrap_or(false);
            if quiet {
                // the sender asked for no notice (or a reaper reports it): a wake must not type it later either
                reg.bus.announce(m.id);
            } else if let (true, Some(target)) = (agent, target) {
                let label = from
                    .map(|f| reg.labels.lock_recover().get(&f).cloned().unwrap_or_else(|| crate::prompts::session_name(f)))
                    .unwrap_or_else(|| crate::prompts::USER_SENDER.into());
                reg.announce(to, target, &m, &label);
            }
            Ok(json!({"id": m.id}))
        }
        "notes.read" | "notes.append" | "notes.export" => {
            let id = p["session"].as_u64().and_then(|x| u32::try_from(x).ok()).ok_or_else(|| missing("session"))?;
            let note =
                note_path(reg, id).ok_or_else(|| axelstra_core::coded!("notes.session_without_file", "this session has no task file"))?;
            match req.method.as_str() {
                "notes.read" => {
                    let name = note.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    Ok(json!({"text": crate::notes::read(&note)?, "path": note.to_string_lossy(), "name": name}))
                }
                "notes.append" => {
                    let title = if p["title"].as_str() == Some("ok") { "ok" } else { "nota" };
                    crate::notes::append(&note, title, p["text"].as_str().unwrap_or_default())?;
                    Ok(json!({"path": note.to_string_lossy()}))
                }
                _ => {
                    // only into a worktree of a registered project, never over a file without an explicit flag
                    let wt = reg.metas.lock_recover().get(&id).and_then(|m| m["worktree"].as_str().map(String::from)).unwrap_or_default();
                    let wt = tokio::task::block_in_place(|| crate::projects::known_worktree(&reg.store, &wt))?;
                    let out = crate::notes::export(&note, &wt, p["overwrite"].as_bool() == Some(true))?;
                    Ok(json!({"path": out.to_string_lossy()}))
                }
            }
        }
        "notes.worktree" => {
            // the task files whose header names this worktree, for the empty state of a worktree without sessions
            let wt = crate::projects::known_worktree(&reg.store, p["worktree"].as_str().unwrap_or_default())?;
            let project = p["project"].as_str().unwrap_or_default().to_string();
            let root = crate::notes::root(reg.saved.parent().unwrap_or(std::path::Path::new("")));
            let items = tokio::task::block_in_place(|| crate::notes::for_worktree(&root, &project, &wt.to_string_lossy()));
            Ok(json!({"items": items}))
        }
        "notes.read_file" => {
            let root = crate::notes::root(reg.saved.parent().unwrap_or(std::path::Path::new("")));
            let project = p["project"].as_str().unwrap_or_default();
            let name = p["name"].as_str().ok_or_else(|| missing("name"))?;
            let note = crate::notes::file_of(&root, project, name)
                .ok_or_else(|| axelstra_core::coded!("notes.not_found", "task file not found"))?;
            Ok(json!({"text": crate::notes::read(&note)?, "path": note.to_string_lossy(), "name": name}))
        }
        "msg.inbox" => {
            let to = p["session"].as_u64().and_then(|x| u32::try_from(x).ok()).ok_or_else(|| missing("session"))?;
            let list = reg.bus.inbox(to, p["all"].as_bool() == Some(true), p["mark"].as_bool() != Some(false));
            Ok(json!(list))
        }
        "msg.wait" => {
            let to = p["session"].as_u64().and_then(|x| u32::try_from(x).ok()).ok_or_else(|| missing("session"))?;
            let types: Vec<String> =
                p["types"].as_array().map(|a| a.iter().filter_map(|t| t.as_str().map(String::from)).collect()).unwrap_or_default();
            let kinds = crate::pullview::wait_kinds(&types)
                .map_err(|k| axelstra_core::coded!("msg.unknown_kind", "unknown message kind {kind}", kind = k))?;
            let wait = Duration::from_millis(p["timeout_ms"].as_u64().unwrap_or(0).min(WAIT_CHUNK_MS));
            Ok(wait_for(reg, to, &kinds, p["mark"].as_bool() != Some(false), wait))
        }
        "msg.thread" => {
            let name = p["name"].as_str().ok_or_else(|| missing("name"))?;
            Ok(json!(reg.bus.thread(name)))
        }
        "msg.reply_of" => {
            let id = p["id"].as_u64().ok_or_else(|| missing("id"))?;
            Ok(json!({"reply": reg.bus.reply_to(id), "original": reg.bus.get(id)}))
        }
        m => Err(unknown_method(m)),
    }
}

/// The longest one `msg.wait` call blocks; `axelstra inbox --wait` calls again for a longer wait.
const WAIT_CHUNK_MS: u64 = 20_000;

/// The unread messages of `kinds` for `session`, at once when some are there, else as soon as one is sent (the bus
/// wakes every waiter on each send) or `wait` runs out: `{messages, timed_out}`. No polling: it sleeps on the bus.
fn wait_for(reg: &Registry, session: u32, kinds: &[&str], mark: bool, wait: Duration) -> Value {
    let end = Instant::now() + wait;
    loop {
        // read the counter first: a message sent between the look and the sleep moves it, so the sleep returns at once
        let seen = reg.bus.sent_count();
        let found = reg.bus.unread_of_kinds(session, kinds, mark);
        let left = end.saturating_duration_since(Instant::now());
        if !found.is_empty() || left.is_zero() || reg.closing.load(Ordering::SeqCst) {
            return json!({"messages": found, "timed_out": found.is_empty()});
        }
        reg.bus.wait_sent(seen, left);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    fn long_sleep() -> Spawn {
        let (prog, args): (&str, Vec<String>) =
            if cfg!(windows) { ("cmd", vec!["/c".into(), "ping -n 30 127.0.0.1 >nul".into()]) } else { ("sleep", vec!["30".into()]) };
        Spawn { program: prog.into(), args, cwd: None, env: vec![], cols: 80, rows: 24 }
    }

    /// Without the session monitor running (nothing else can announce), a message must reach every watcher at once.
    #[test]
    fn a_message_announces_itself_to_watchers_without_the_monitor() {
        let dir = std::env::temp_dir().join(format!("axl-msg-announce-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let reg = Arc::new(Registry::new(dir.join("projects.json"), "p".into(), "t".into()));
        let (asker, boss) = (reg.spawn(&long_sleep(), None, None).unwrap(), reg.spawn(&long_sleep(), None, None).unwrap());
        let (tx, mut rx) = mpsc::channel::<Vec<u8>>(16);
        reg.watchers.lock_recover().push((Arc::new(AtomicBool::new(false)), tx));

        let send = |from: u32, to: u32, kind: &str, reply_to: Option<u64>| {
            let req = Req { id: 1, method: "msg.send".into(), params: Value::Null };
            let p = json!({"from": from, "to": to, "text": "t", "kind": kind, "reply_to": reply_to, "quiet": true});
            let (tx2, _rx2) = mpsc::channel(1);
            let (mut subs, mut authed, mut cli, mut agent, mut after, mut delay) =
                (HashMap::new(), true, true, None, vec![], Duration::ZERO);
            let mut c = Ctx {
                req: &req,
                p: &p,
                reg: &reg,
                tx: &tx2,
                subs: &mut subs,
                authed: &mut authed,
                cli: &mut cli,
                agent: &mut agent,
                peer: None,
                token: "t",
                after: &mut after,
                delay: &mut delay,
            };
            call(&mut c).unwrap()["id"].as_u64().unwrap()
        };
        let q = send(asker, boss, "ask", None);
        assert!(rx.try_recv().is_ok(), "the ask is announced");
        while rx.try_recv().is_ok() {}
        send(boss, asker, "reply", Some(q));
        let frame = rx.try_recv().expect("the reply is announced by the handler itself, in the same call");
        assert!(String::from_utf8_lossy(&frame).contains("session.changed"));
        assert_eq!(reg.bus.reply_to(q).map(|m| m.text), Some("t".to_string()));
    }
}
