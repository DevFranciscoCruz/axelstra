use super::*;

pub(super) fn call(c: &mut Ctx) -> Result<Value> {
    let req = c.req;
    let (p, reg, tx) = (c.p, c.reg, c.tx);
    let subs = &mut *c.subs;
    match req.method.as_str() {
        "fs.list" | "fs.read" | "fs.stat" | "fs.write" | "fs.search" | "fs.grep" | "fs.create" | "fs.rename" | "fs.delete" => {
            let wt = crate::projects::known_worktree(&reg.store, p["worktree"].as_str().unwrap_or_default())?;
            let path = p["path"].as_str().unwrap_or_default();
            tokio::task::block_in_place(|| -> Result<Value> {
                use crate::fsops;
                Ok(match req.method.as_str() {
                    "fs.list" => json!({"entries": fsops::list(&wt, path)?}),
                    "fs.grep" => json!({"matches": fsops::grep(&wt, p["query"].as_str().unwrap_or_default())?}),
                    "fs.create" => {
                        fsops::create(&wt, path, p["dir"].as_bool().unwrap_or(false))?;
                        json!({})
                    }
                    "fs.rename" => {
                        fsops::rename(&wt, path, p["to"].as_str().unwrap_or_default())?;
                        json!({})
                    }
                    "fs.delete" => {
                        fsops::delete(&wt, path)?;
                        json!({})
                    }
                    "fs.search" => json!({"files": fsops::search(&wt, p["query"].as_str().unwrap_or_default())?}),
                    "fs.read" => serde_json::to_value(fsops::read(&wt, path)?)?,
                    "fs.stat" => json!({"hash": fsops::stat_hash(&wt, path)?}),
                    _ => {
                        let text = p["text"].as_str().ok_or_else(|| anyhow::anyhow!("missing text"))?;
                        let eol = p["eol"].as_str().unwrap_or("lf");
                        let bom = p["bom"].as_bool().unwrap_or(false);
                        match fsops::write(&wt, path, text, eol, bom, p["base_hash"].as_str())? {
                            fsops::Saved::Ok { hash } => json!({"hash": hash}),
                            fsops::Saved::Conflict { disk_hash } => json!({"conflict": true, "disk_hash": disk_hash}),
                        }
                    }
                })
            })
        }
        // one watch per connection (key u32::MAX in `subs`); a new subscribe replaces the old one
        "watch.subscribe" => {
            ensure_session_notifier(reg, tx, subs);
            let wt = crate::projects::known_worktree(&reg.store, p["worktree"].as_str().unwrap_or_default())?;
            if let Some(old) = subs.remove(&u32::MAX) {
                old.store(true, Ordering::SeqCst);
            }
            let stop = Arc::new(AtomicBool::new(false));
            let (tx2, path) = (tx.clone(), wt.to_string_lossy().to_string());
            let weak = Arc::downgrade(reg);
            tokio::task::block_in_place(|| {
                crate::watcher::start(wt, stop.clone(), move || {
                    // the worktree itself was deleted (outside the app): its sessions have nothing left to show
                    if !std::path::Path::new(&path).exists() {
                        if let Some(r) = weak.upgrade() {
                            reap_orphans(&r);
                            r.projects_changed();
                        }
                    }
                    let ev = json!({"event": "fs.changed", "worktree": path});
                    let _ = tx2.blocking_send(encode(EVENT, ev.to_string().as_bytes()));
                })
            })?;
            subs.insert(u32::MAX, stop);
            Ok(json!({}))
        }
        "watch.unsubscribe" => {
            ensure_session_notifier(reg, tx, subs);
            if let Some(old) = subs.remove(&u32::MAX) {
                old.store(true, Ordering::SeqCst);
            }
            Ok(json!({}))
        }
        "open.editor" => {
            let wt = crate::projects::known_worktree(&reg.store, p["worktree"].as_str().unwrap_or_default())?;
            // only editors we know; never a program name from the client, and never through the shell
            let (prog, args) = match p["app"].as_str().unwrap_or_default() {
                "code" => crate::fsops::code_launch(&wt)?,
                other => return Err(axelstra_core::coded!("fs.unknown_editor", "unknown editor {editor:?}", editor = other)),
            };
            axelstra_core::git::quiet(&mut std::process::Command::new(&prog)).args(args).spawn().map_err(|e| {
                axelstra_core::coded!("fs.editor_start_failed", "could not start {program}: {detail}", program = prog.display(), detail = e)
            })?;
            Ok(json!({}))
        }
        // The app opens an HTML path clicked in a terminal. Resolution and confinement: `htmlpath`. Not for agents: opening a
        // browser is the user's act (the host command calls this itself, so the page never passes a url to the opener).
        "files.resolve_html" => {
            let id = p["session"].as_u64().and_then(|x| u32::try_from(x).ok()).ok_or_else(|| missing("session"))?;
            reg.get(id).ok_or_else(|| axelstra_core::coded!("session.not_found", "no such session {id}", id = id))?;
            let meta = reg.metas.lock_recover().get(&id).cloned().unwrap_or(Value::Null);
            let cwd = reg.specs.lock_recover().get(&id).and_then(|s| s["cwd"].as_str().map(std::path::PathBuf::from));
            let mut roots: Vec<std::path::PathBuf> = cwd.iter().cloned().collect();
            roots.extend(meta["worktree"].as_str().filter(|w| !w.is_empty()).map(std::path::PathBuf::from));
            if let Some(proj) = meta["project"].as_str().and_then(|id| crate::projects::find(&reg.store, id).ok()) {
                roots.push(proj.path.into());
            }
            let raw = p["path"].as_str().unwrap_or_default().to_string();
            let r = tokio::task::block_in_place(|| crate::htmlpath::resolve(&raw, cwd.as_deref(), &roots))?;
            Ok(json!({"path": r.path, "url": r.url}))
        }
        m => anyhow::bail!("unknown method {m}"),
    }
}
