use super::*;

pub(super) fn call(c: &mut Ctx) -> Result<Value> {
    let req = c.req;
    let (p, reg) = (c.p, c.reg);
    match req.method.as_str() {
        "git.changes" => {
            let wt = crate::projects::known_worktree(&reg.store, p["worktree"].as_str().unwrap_or_default())?;
            let base = p["base"].as_str();
            // dropped when the watcher of this worktree reports a file change (it bumps the path), by any mutation and by a ref move
            let path = norm(&wt.to_string_lossy());
            crate::memo::memo().cached(
                &memo_key(reg, &["changes", &path, base.unwrap_or_default()]),
                memo_opts(reg, &["*", "wt", &path], Some(crate::memo::WORKTREE_MAX_AGE)),
                || {
                    Ok(serde_json::to_value(tokio::task::block_in_place(|| {
                        crate::memo::git_pool().run(|| crate::changes::changes(&wt, base))
                    })?)?)
                },
            )
        }
        "git.log" | "git.show" => {
            let wt = crate::projects::known_worktree(&reg.store, p["worktree"].as_str().unwrap_or_default())?;
            tokio::task::block_in_place(|| -> Result<Value> {
                Ok(if req.method == "git.log" {
                    json!({"commits": crate::changes::log(&wt, p["base"].as_str())?})
                } else {
                    json!({"diff": crate::changes::show(&wt, p["sha"].as_str().unwrap_or_default())?})
                })
            })
        }
        "git.diff" => {
            let wt = crate::projects::known_worktree(&reg.store, p["worktree"].as_str().unwrap_or_default())?;
            let path = p["path"].as_str().ok_or_else(|| missing("path"))?;
            let base = p["base"].as_str();
            let text = tokio::task::block_in_place(|| crate::changes::diff(&wt, path, base))?;
            Ok(json!({"diff": text}))
        }
        "git.suggest_pr" => {
            let wt = crate::projects::known_worktree(&reg.store, p["worktree"].as_str().unwrap_or_default())?;
            let base = p["base"].as_str().map(String::from);
            let t = tokio::task::block_in_place(|| crate::publish::suggest_pr(&wt, base.as_deref()))?;
            Ok(serde_json::to_value(t)?)
        }
        "git.commit" | "git.push" | "git.pr" | "git.suggest" => {
            let wt = crate::projects::known_worktree(&reg.store, p["worktree"].as_str().unwrap_or_default())?;
            let s = |k: &str| p[k].as_str().unwrap_or_default().to_string();
            tokio::task::block_in_place(|| -> Result<Value> {
                use crate::publish;
                Ok(match req.method.as_str() {
                    "git.commit" => serde_json::to_value(publish::commit(&wt, &s("message"))?)?,
                    "git.push" => json!({"output": publish::push(&wt)?}),
                    "git.suggest" => json!({"message": publish::suggest_message(&wt)?}),
                    _ => json!({"url": publish::pull_request(&wt, &s("title"), &s("body"), p["base"].as_str())?}),
                })
            })
        }
        "git.timeline" => {
            // read-only, so agents may ask for it too
            let wt = crate::projects::known_worktree(&reg.store, p["worktree"].as_str().unwrap_or_default())?;
            let project = p["project"].as_str().unwrap_or_default().to_string();
            let with_ci = p["ci"].as_bool().unwrap_or(false);
            let root = crate::notes::root(reg.saved.parent().unwrap_or(std::path::Path::new("")));
            tokio::task::block_in_place(|| -> Result<Value> {
                let notes = crate::notes::for_worktree(&root, &project, &wt.to_string_lossy());
                Ok(serde_json::to_value(crate::timeline::build(&wt, &notes, with_ci)?)?)
            })
        }
        "git.checks" | "git.merge" => {
            let wt = crate::projects::known_worktree(&reg.store, p["worktree"].as_str().unwrap_or_default())?;
            let out = tokio::task::block_in_place(|| -> Result<Value> {
                use crate::checks;
                Ok(if req.method == "git.checks" {
                    let st = checks::status(&wt);
                    // the daemon keeps this worktree's PR fresh from now on, and tells the other panels if it moved
                    super::triggers::seen_by_app(reg, &wt, &st);
                    serde_json::to_value(&st)?
                } else {
                    json!({"output": checks::merge(&wt, p["auto"].as_bool().unwrap_or(false), p["delete_branch"].as_bool().unwrap_or(false))?})
                })
            });
            if req.method == "git.merge" && out.is_ok() {
                sweep_soon(reg);
            }
            out
        }
        // the window got focus: PRs that GitHub changed meanwhile raise no local event, so look at the stale ones once
        "gh.pr_refresh" => {
            crate::memo::memo().bump("gh");
            crate::prwatch::WAKE.bump();
            Ok(json!({}))
        }
        "gh.list" => {
            let pr = crate::projects::find(&reg.store, p["project"].as_str().unwrap_or_default())?;
            let kind = p["kind"].as_str().unwrap_or("pr").to_string();
            // PR and issue lists move on GitHub's side: `pr.changed` and the window's `gh.pr_refresh` bump `gh`
            crate::memo::memo().cached(
                &memo_key(reg, &["gh.list", &pr.id, &kind]),
                memo_opts(reg, &["gh", "wt"], Some(crate::memo::GH_MAX_AGE)),
                || {
                    let items = tokio::task::block_in_place(|| {
                        crate::memo::git_pool().run(|| crate::gh::list(std::path::Path::new(&pr.path), &kind))
                    })?;
                    Ok(json!({"items": items}))
                },
            )
        }
        "gh.comments" => {
            let pr = crate::projects::find(&reg.store, p["project"].as_str().unwrap_or_default())?;
            let n = p["number"].as_u64().ok_or_else(|| missing("number"))?;
            let (comments, reviews) = tokio::task::block_in_place(|| crate::gh::review_comments(std::path::Path::new(&pr.path), n))?;
            Ok(json!({"comments": comments, "reviews": reviews}))
        }
        "gh.send_comment" => {
            let wt = crate::projects::known_worktree(&reg.store, p["worktree"].as_str().unwrap_or_default())?;
            crate::projects::find(&reg.store, p["project"].as_str().unwrap_or_default())?;
            let n = p["number"].as_u64().ok_or_else(|| missing("number"))?;
            let id = p["comment_id"].as_u64().ok_or_else(|| missing("comment_id"))?;
            // the lead is looked up first: no point asking GitHub when nobody can take the task
            let dry = p["dry_run"].as_bool() == Some(true);
            let lead = find_lead(reg, &wt.to_string_lossy());
            if lead.is_none() && !dry {
                return Err(no_lead());
            }
            // the text is built here from what GitHub says now; nothing the client sent is forwarded
            let c = tokio::task::block_in_place(|| crate::gh::fetch_comment(&wt, n, id))?;
            let text = crate::gh::comment_task(n, &c);
            let Some(lead) = lead.filter(|_| !dry) else { return Ok(json!({"text": text, "sent": false})) };
            // a hibernated lead wakes and is told when it is back
            reg.hib.touch(lead);
            if reg.hib.is_sleeping(lead) {
                let m = reg.bus.send(None, lead, "message", &text, None, None);
                hibernate::wake(reg, lead);
                return Ok(json!({"text": text, "sent": true, "session": lead, "message": m.id}));
            }
            let target = reg.get(lead).ok_or_else(no_lead)?;
            let m = reg.bus.send(None, lead, "message", &text, None, None);
            reg.announce(lead, target, &m, crate::prompts::REVIEW_SENDER);
            Ok(json!({"text": text, "sent": true, "session": lead, "message": m.id}))
        }
        m => Err(unknown_method(m)),
    }
}

fn no_lead() -> anyhow::Error {
    axelstra_core::coded!("worktree.no_lead", "no live lead in this worktree: start its main agent first")
}

/// The running lead session (meta `lead: true`) of the worktree at `path`.
fn find_lead(reg: &Registry, path: &str) -> Option<u32> {
    // one lock at a time, sessions before metas (the registry's order)
    let mut alive: Vec<u32> = reg.sessions.lock_recover().iter().filter(|(_, s)| s.try_wait().is_none()).map(|(id, _)| *id).collect();
    alive.extend(reg.hib.sleepers().into_iter().map(|(id, _)| id));
    let metas = reg.metas.lock_recover();
    alive
        .into_iter()
        .filter(|id| {
            metas.get(id).is_some_and(|m| {
                m["lead"] == true && m["kind"] == "agent" && m["worktree"].as_str().is_some_and(|w| axelstra_core::paths::same(w, path))
            })
        })
        .min()
}
