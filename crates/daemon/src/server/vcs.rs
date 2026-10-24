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
        "git.checks" | "git.merge" => {
            let wt = crate::projects::known_worktree(&reg.store, p["worktree"].as_str().unwrap_or_default())?;
            tokio::task::block_in_place(|| -> Result<Value> {
                use crate::checks;
                Ok(if req.method == "git.checks" {
                    serde_json::to_value(checks::status(&wt))?
                } else {
                    json!({"output": checks::merge(&wt, p["auto"].as_bool().unwrap_or(false), p["delete_branch"].as_bool().unwrap_or(false))?})
                })
            })
        }
        "gh.list" => {
            let pr = crate::projects::find(&reg.store, p["project"].as_str().unwrap_or_default())?;
            let kind = p["kind"].as_str().unwrap_or("pr").to_string();
            let items = tokio::task::block_in_place(|| crate::gh::list(std::path::Path::new(&pr.path), &kind))?;
            Ok(json!({"items": items}))
        }
        m => anyhow::bail!("unknown method {m}"),
    }
}
