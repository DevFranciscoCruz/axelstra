use super::*;

pub(super) fn call(c: &mut Ctx) -> Result<Value> {
    let req = c.req;
    let (p, reg) = (c.p, c.reg);
    match req.method.as_str() {
        "git.changes" => {
            let wt = crate::projects::known_worktree(&reg.store, p["worktree"].as_str().unwrap_or_default())?;
            let base = p["base"].as_str();
            Ok(serde_json::to_value(tokio::task::block_in_place(|| crate::changes::changes(&wt, base))?)?)
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
            let path = p["path"].as_str().ok_or_else(|| anyhow::anyhow!("missing path"))?;
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
                    serde_json::to_value(checks::status(&wt))?
                } else {
                    json!({"output": checks::merge(&wt, p["auto"].as_bool().unwrap_or(false), p["delete_branch"].as_bool().unwrap_or(false))?})
                })
            });
            if req.method == "git.merge" && out.is_ok() {
                sweep_soon(reg);
            }
            out
        }
        "gh.list" => {
            let pr = crate::projects::find(&reg.store, p["project"].as_str().unwrap_or_default())?;
            let kind = p["kind"].as_str().unwrap_or("pr").to_string();
            let items = tokio::task::block_in_place(|| crate::gh::list(std::path::Path::new(&pr.path), &kind))?;
            Ok(json!({"items": items}))
        }
        "gh.comments" => {
            let pr = crate::projects::find(&reg.store, p["project"].as_str().unwrap_or_default())?;
            let n = p["number"].as_u64().ok_or_else(|| anyhow::anyhow!("missing number"))?;
            let (comments, reviews) = tokio::task::block_in_place(|| crate::gh::review_comments(std::path::Path::new(&pr.path), n))?;
            Ok(json!({"comments": comments, "reviews": reviews}))
        }
        "gh.send_comment" => {
            let wt = crate::projects::known_worktree(&reg.store, p["worktree"].as_str().unwrap_or_default())?;
            crate::projects::find(&reg.store, p["project"].as_str().unwrap_or_default())?;
            let n = p["number"].as_u64().ok_or_else(|| anyhow::anyhow!("missing number"))?;
            let id = p["comment_id"].as_u64().ok_or_else(|| anyhow::anyhow!("missing comment_id"))?;
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
            let target = reg.get(lead).ok_or_else(no_lead)?;
            let m = reg.bus.send(None, lead, "message", &text, None, None);
            reg.deliver_untrusted(lead, target, &crate::msgbus::notice(&m, "o utilizador (comentário de review)"));
            Ok(json!({"text": text, "sent": true, "session": lead, "message": m.id}))
        }
        m => anyhow::bail!("unknown method {m}"),
    }
}

fn no_lead() -> anyhow::Error {
    axelstra_core::coded!("worktree.no_lead", "no live lead in this worktree: start its main agent first")
}

/// The running lead session (meta `lead: true`) of the worktree at `path`.
fn find_lead(reg: &Registry, path: &str) -> Option<u32> {
    // one lock at a time, sessions before metas (the registry's order)
    let alive: Vec<u32> = reg.sessions.lock_recover().iter().filter(|(_, s)| s.try_wait().is_none()).map(|(id, _)| *id).collect();
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
