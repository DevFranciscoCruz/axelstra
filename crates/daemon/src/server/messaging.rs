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
            let target = reg.get(to).ok_or_else(|| axelstra_core::coded!("session.not_found", "no such session {id}", id = to))?;
            let text = p["text"].as_str().unwrap_or_default();
            let m = reg.bus.send(from, to, kind, text, p["reply_to"].as_u64(), p["thread"].as_str().map(String::from));
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
            let quiet = p["quiet"].as_bool() == Some(true) || (kind == "done" && from.map(|f| reg.bus.is_reaped(f)).unwrap_or(false));
            let agent = reg.metas.lock_recover().get(&to).map(|m| m["kind"] == "agent").unwrap_or(false);
            if agent && !quiet {
                let label = from
                    .map(|f| reg.labels.lock_recover().get(&f).cloned().unwrap_or_else(|| crate::prompts::session_name(f)))
                    .unwrap_or_else(|| crate::prompts::USER_SENDER.into());
                reg.deliver_untrusted(to, target.clone(), &crate::msgbus::notice(&m, &label));
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
        "msg.thread" => {
            let name = p["name"].as_str().ok_or_else(|| missing("name"))?;
            Ok(json!(reg.bus.thread(name)))
        }
        "msg.reply_of" => {
            let id = p["id"].as_u64().ok_or_else(|| missing("id"))?;
            Ok(json!({"reply": reg.bus.reply_to(id), "original": reg.bus.get(id)}))
        }
        m => anyhow::bail!("unknown method {m}"),
    }
}
