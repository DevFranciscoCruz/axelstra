use super::*;

pub(super) fn call(c: &mut Ctx) -> Result<Value> {
    let req = c.req;
    let (p, reg) = (c.p, c.reg);
    match req.method.as_str() {
        "worktree.overview" => Ok(tokio::task::block_in_place(|| crate::overview::cached(&reg.store, &|path| reg.worktree_busy(path)))),
        "worktree.sweep" => {
            let dry = p["dry_run"].as_bool().unwrap_or(false);
            let before = reg.closed.read();
            let done = tokio::task::block_in_place(|| {
                let busy = |path: &str| reg.worktree_busy(path) || reg.worktree_pending(path);
                let close = |path: &str| reg.close_worktree(path);
                let touched = |path: &str| reg.worktree_touched(path);
                let env = crate::janitor::Env { busy: &busy, close: &close, touched: &touched };
                let mut v = crate::janitor::sweep_with(&reg.store, &env, dry);
                v.extend(reg.clean_leftovers(dry));
                // the user asks for a clean-up: worktrees of a removed project that were held get another try
                if !dry && crate::removal::has_pending(&reg.store) {
                    crate::removal::retry(&reg.store, &close);
                }
                v
            });
            let after = reg.closed.read();
            if !dry {
                reg.persist();
                reg.projects_changed();
            }
            reg.announce_events();
            Ok(
                json!({"swept": done, "closed_sessions": after.0 - before.0, "closed_procs": after.1 - before.1, "freed_bytes": after.2 - before.2}),
            )
        }
        // worktrees the automatic sweep removed since the last call (the app turns them into a toast)
        "janitor.events" => Ok(json!({"events": crate::janitor::take_events()})),
        "janitor.auto" => {
            // on unless the user turned it off: the file records the opt-out
            let flag = reg.saved.with_file_name("janitor.off");
            if let Some(on) = p["on"].as_bool() {
                if on {
                    let _ = std::fs::remove_file(&flag);
                } else {
                    let _ = std::fs::write(&flag, "1");
                }
            }
            Ok(json!({"on": !flag.exists()}))
        }
        "dispatch.meta" => {
            // titles and archive flags the dispatcher keeps per worktree (see `axelstra dispatch`)
            let f = reg.saved.with_file_name("dispatch.json");
            Ok(std::fs::read_to_string(f).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()).unwrap_or_else(|| json!({})))
        }
        "worktree.status" => {
            // only paths the daemon knows; the list comes from the UI and git runs in each
            let list: Vec<String> =
                p["paths"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default();
            let norm = |s: &str| s.replace('\\', "/").to_ascii_lowercase();
            let out = tokio::task::block_in_place(|| {
                let mut allowed = std::collections::HashSet::new();
                for pr in crate::projects::load(&reg.store) {
                    for w in axelstra_core::git::worktree_list(std::path::Path::new(&pr.path)).unwrap_or_default() {
                        allowed.insert(norm(&w.path));
                    }
                }
                list.iter().take(64).filter(|w| allowed.contains(&norm(w))).map(|w| crate::wtstatus::status_cached(w)).collect::<Vec<_>>()
            });
            Ok(json!({"status": out}))
        }
        "project.list" => {
            if let Some((at, v)) = reg.plist.lock_recover().as_ref() {
                if at.elapsed() < Duration::from_secs(3) {
                    return Ok(v.clone());
                }
            }
            // the app asks for the list when something changed in the repositories: the moment to notice a worktree deleted behind our back
            reap_orphans(reg);
            let gen = reg.wt_gen.load(std::sync::atomic::Ordering::SeqCst);
            let ps = crate::projects::load(&reg.store);
            let v: Vec<Value> = ps
                .iter()
                .map(|pr| {
                    let repo = std::path::Path::new(&pr.path);
                    let mut wts = axelstra_core::git::worktree_list(repo).unwrap_or_default();
                    wts.retain(|w| w.is_main || w.path.replace('\\', "/").contains("/.axelstra-wt/"));
                    let ident = axelstra_core::git::identity(repo);
                    let parents = crate::projects::parent_paths(pr, &wts);
                    let wts: Vec<Value> = wts
                        .into_iter()
                        .map(|w| {
                            let parent = parents.get(&w.path).cloned();
                            let mut v = serde_json::to_value(w).unwrap_or_default();
                            if let Some(par) = parent {
                                v["parent"] = json!(par);
                            }
                            v
                        })
                        .collect();
                    json!({"id": pr.id, "name": ident.display_name, "path": pr.path, "worktrees": wts, "git": axelstra_core::git::is_repo(repo),
                           "setup": pr.setup, "default_branch": axelstra_core::git::default_branch(repo), "slug": ident.slug, "icon_url": ident.icon_url, "remote": ident.remote})
                })
                .collect();
            let all = Value::Array(v);
            // an answer computed while a project or worktree changed may predate the change: serve it, do not keep it
            if reg.wt_gen.load(std::sync::atomic::Ordering::SeqCst) == gen {
                *reg.plist.lock_recover() = Some((std::time::Instant::now(), all.clone()));
            }
            Ok(all)
        }
        "project.logo" => {
            let pr = crate::projects::find(&reg.store, p["id"].as_str().unwrap_or_default())?;
            let logo = tokio::task::block_in_place(|| crate::projects::logo_data_url(std::path::Path::new(&pr.path)));
            Ok(json!({"data_url": logo}))
        }
        "project.add" => {
            let path = p["path"].as_str().ok_or_else(|| missing("path"))?;
            Ok(serde_json::to_value(crate::projects::add(&reg.store, path)?)?)
        }
        "project.setup" => {
            let id = p["id"].as_str().ok_or_else(|| missing("id"))?;
            crate::projects::set_setup(&reg.store, id, p["setup"].as_str().map(String::from))?;
            Ok(json!({}))
        }
        "pm.detect" => {
            let path =
                p["id"].as_str().and_then(|id| crate::projects::find(&reg.store, id).ok()).map(|pr| std::path::PathBuf::from(pr.path));
            let preferred = read_settings(&reg.saved)["pm"].as_str().map(String::from);
            let (installed, suggested) =
                tokio::task::block_in_place(|| crate::projects::package_managers(path.as_deref(), preferred.as_deref()));
            let has_package_json = path.as_ref().map(|p| p.join("package.json").is_file()).unwrap_or(false);
            Ok(json!({"installed": installed, "suggested": suggested, "package_json": has_package_json}))
        }
        "project.remove" | "project.remove_preview" => super::removal::call(c),
        "worktree.create" => {
            let pr = crate::projects::find(&reg.store, p["project"].as_str().unwrap_or_default())?;
            let name = p["name"].as_str().ok_or_else(|| missing("name"))?;
            // a parent that is not a worktree of this project is refused before anything is created
            let parent = p["parent"].as_str().map(str::trim).filter(|s| !s.is_empty());
            if let Some(par) = parent {
                crate::projects::check_parent(&pr, None, par)?;
            }
            let (wt, env) = tokio::task::block_in_place(|| crate::projects::worktree_create_report(&pr, name))?;
            // also clears a link left by an earlier worktree at the same path
            if let Err(e) = crate::projects::set_parent(&reg.store, &pr.id, &wt.path, parent) {
                eprintln!("parent of {}: {e:#}", wt.path);
            }
            // made by hand in the app (an agent's CLI does not say so): the janitor treats it with more care
            if p["manual"].as_bool() == Some(true) {
                if let Err(e) = crate::projects::set_manual(&reg.store, &pr.id, &wt.path) {
                    eprintln!("manual mark of {}: {e:#}", wt.path);
                }
                reg.note_activity(&wt.path);
            }
            // made by Axelstra's own flow (`dispatch new`): the janitor may remove it once integrated or empty
            if p["auto"].as_bool() == Some(true) && p["manual"].as_bool() != Some(true) {
                if let Err(e) = crate::projects::set_auto(&reg.store, &pr.id, &wt.path) {
                    eprintln!("auto mark of {}: {e:#}", wt.path);
                }
            }
            // the worktree plus what .worktreeinclude copied (env_copied) and left out (env_skipped)
            let mut v = serde_json::to_value(wt)?;
            v["env_copied"] = json!(env.copied);
            v["env_skipped"] = json!(env.skipped);
            Ok(v)
        }
        "worktree.set_parent" => {
            let pr = crate::projects::find(&reg.store, p["project"].as_str().unwrap_or_default())?;
            let path = p["path"].as_str().ok_or_else(|| missing("path"))?;
            let parent = p["parent"].as_str();
            tokio::task::block_in_place(|| crate::projects::set_parent(&reg.store, &pr.id, path, parent))?;
            Ok(json!({}))
        }
        "worktree.rename" => {
            let pr = crate::projects::find(&reg.store, p["project"].as_str().unwrap_or_default())?;
            let path = p["path"].as_str().ok_or_else(|| missing("path"))?;
            let name = p["name"].as_str().ok_or_else(|| missing("name"))?;
            let busy = reg.worktree_working(path);
            let wt = tokio::task::block_in_place(|| crate::projects::worktree_rename(&pr, path, name, busy))?;
            let _ = crate::projects::rename_parent(&reg.store, &pr.id, path, &wt.path);
            // the dispatcher's notes (title, requests) are keyed by path: they follow the folder
            let f = reg.saved.with_file_name("dispatch.json");
            let norm = |s: &str| s.replace('\\', "/").trim_end_matches('/').to_ascii_lowercase();
            if let Some(mut meta) = std::fs::read_to_string(&f).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()) {
                if let Some(e) = meta.as_object_mut().and_then(|m| m.remove(&norm(path))) {
                    meta[norm(&wt.path)] = e;
                    write_atomic(&f, &serde_json::to_vec_pretty(&meta)?)?;
                }
            }
            Ok(serde_json::to_value(wt)?)
        }
        "project.env_preview" => {
            // what .worktreeinclude would copy into the next worktree, for the project settings dialog
            let pr = crate::projects::find(&reg.store, p["project"].as_str().unwrap_or_default())?;
            Ok(serde_json::to_value(tokio::task::block_in_place(|| crate::wtenv::plan(std::path::Path::new(&pr.path)))?)?)
        }
        "dispatch.overlaps" => {
            let f = reg.saved.with_file_name("dispatch.json");
            Ok(tokio::task::block_in_place(|| crate::overlaps::overlaps(&reg.store, &f)))
        }
        "dispatch.overlap_files" => {
            let f = reg.saved.with_file_name("dispatch.json");
            Ok(tokio::task::block_in_place(|| crate::overlaps::overlap_files(&reg.store, &f)))
        }
        "worktree.rebase" => {
            let pr = crate::projects::find(&reg.store, p["project"].as_str().unwrap_or_default())?;
            let path = p["path"].as_str().ok_or_else(|| missing("path"))?;
            let busy = reg.worktree_working(path);
            tokio::task::block_in_place(|| crate::rebase::rebase(std::path::Path::new(&pr.path), path, busy))
        }
        "ports.list" => {
            let wt = crate::projects::known_worktree(&reg.store, p["worktree"].as_str().unwrap_or_default())?;
            let norm = |s: &str| s.replace('\\', "/").to_ascii_lowercase();
            let key = norm(&wt.to_string_lossy());
            let ids: Vec<u32> = {
                let metas = reg.metas.lock_recover();
                metas.iter().filter(|(_, m)| m["worktree"].as_str().map(norm) == Some(key.clone())).map(|(id, _)| *id).collect()
            };
            let pids: Vec<u32> =
                ids.iter().filter_map(|id| reg.get(*id)).filter(|s| s.try_wait().is_none()).filter_map(|s| s.pid()).collect();
            Ok(json!({"ports": tokio::task::block_in_place(|| crate::ports::listening(&pids))?}))
        }
        "worktree.from_pr" => {
            let pr = crate::projects::find(&reg.store, p["project"].as_str().unwrap_or_default())?;
            let n = p["number"].as_u64().ok_or_else(|| missing("number"))?;
            let (wt, warning) = tokio::task::block_in_place(|| crate::projects::worktree_from_pr_report(&pr, n))?;
            let mut v = serde_json::to_value(wt)?;
            if let (Some(w), Some(o)) = (warning, v.as_object_mut()) {
                o.insert("warning".into(), json!(w.text));
                o.insert("warning_code".into(), json!(w.code));
                o.insert("warning_params".into(), w.params_json());
            }
            Ok(v)
        }
        "project.integrate" => {
            let id = p["id"].as_str().ok_or_else(|| missing("id"))?;
            crate::projects::set_integrate(&reg.store, id, p["auto"].as_bool(), p["push"].as_bool())?;
            Ok(json!({}))
        }
        // the user asks for the integration of a finished worktree by hand (also when the project's automatic one is off)
        "worktree.integrate" => {
            let pr = crate::projects::find(&reg.store, p["project"].as_str().unwrap_or_default())?;
            let path = p["path"].as_str().ok_or_else(|| missing("path"))?;
            let out = tokio::task::block_in_place(|| {
                let wt = axelstra_core::git::worktree_list(std::path::Path::new(&pr.path))?
                    .into_iter()
                    .find(|w| crate::projects::norm_path(&w.path) == crate::projects::norm_path(path))
                    .ok_or_else(|| axelstra_core::coded!("worktree.not_found", "not a worktree of this project"))?;
                let branch = wt.branch.clone().unwrap_or_default();
                if reg.worktree_busy(&wt.path) {
                    return Err(axelstra_core::coded!("integrate.busy", "an agent is still working in {branch}", branch = branch));
                }
                let r = crate::integrate::integrate_with(&pr, &wt, &|| reg.close_worktree(&wt.path));
                match &r {
                    Err(e) => crate::integrate::set_attention(&wt.path, crate::integrate::as_coded(e, &branch)),
                    Ok(_) => crate::integrate::clear_attention(&wt.path),
                }
                r
            })?;
            forget_project_list(reg);
            // sessions saved for the folder that just went are dropped from sessions.json
            reg.persist();
            Ok(match out {
                crate::integrate::Outcome::Nothing => {
                    json!({"integrated": false, "into": "", "how": "none", "removed": false, "pushed": null})
                }
                crate::integrate::Outcome::Integrated { into, how, pushed, removed, push_error, closed } => {
                    let mut v =
                        json!({"integrated": true, "into": into, "how": how, "removed": removed, "pushed": pushed, "closed": closed});
                    if let Some(c) = push_error {
                        v["push_code"] = json!(c.code);
                        v["push_params"] = c.params_json();
                        v["push_text"] = json!(c.text);
                    }
                    v
                }
            })
        }
        "worktree.remove" => {
            let pr = crate::projects::find(&reg.store, p["project"].as_str().unwrap_or_default())?;
            let path = p["path"].as_str().ok_or_else(|| missing("path"))?;
            // refuse before touching any session: the main worktree (or a stranger path) must keep its terminals
            let known = axelstra_core::git::worktree_list(std::path::Path::new(&pr.path))?;
            let gone = crate::projects::norm_path(path);
            match known.iter().find(|w| crate::projects::norm_path(&w.path) == gone) {
                None => return Err(axelstra_core::coded!("worktree.not_found", "not a worktree of this project")),
                Some(w) if w.is_main => return Err(axelstra_core::coded!("worktree.remove_main", "cannot remove the main worktree")),
                _ => {}
            }
            // a live session keeps its cwd open, which on Windows makes the folder impossible to remove: kill and
            // wait for them first
            let doomed: Vec<_> = {
                let sessions = reg.sessions.lock_recover();
                let metas = reg.metas.lock_recover();
                sessions
                    .iter()
                    .filter(|(id, _)| {
                        metas.get(id).and_then(|m| m["worktree"].as_str()).is_some_and(|w| crate::projects::norm_path(w) == gone)
                    })
                    .map(|(_, s)| s.clone())
                    .collect()
            };
            tokio::task::block_in_place(|| kill_and_wait(&doomed, Duration::from_secs(5)));
            tokio::task::block_in_place(|| crate::projects::worktree_remove(&pr, path))?;
            let _ = crate::projects::forget_parent(&reg.store, &pr.id, path);
            reg.persist();
            Ok(json!({}))
        }
        m => anyhow::bail!("unknown method {m}"),
    }
}
