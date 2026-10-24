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
            tokio::task::block_in_place(|| {
                crate::watcher::start(wt, stop.clone(), move || {
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
        m => anyhow::bail!("unknown method {m}"),
    }
}
