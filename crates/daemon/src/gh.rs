//! Pull requests and issues of a project through the GitHub CLI. Read-only; nothing here writes to GitHub.

use anyhow::{bail, Context, Result};
use axelstra_core::git;
use serde::Serialize;
use serde_json::Value;
use std::path::Path;
use std::process::Command;

#[derive(Serialize, Debug, PartialEq)]
pub struct Item {
    pub number: u64,
    pub title: String,
    pub author: String,
    pub url: String,
    pub state: String,
    pub draft: bool,
    /// PR head branch; empty for issues
    pub branch: String,
    /// "pass" | "fail" | "pending" | "" (no checks)
    pub checks: String,
    /// "approved" | "changes" | "review" | ""
    pub review: String,
}

/// One word for a PR's checks: any failure wins, then anything still running.
pub fn summarize_checks(rollup: &Value) -> &'static str {
    let Some(list) = rollup.as_array().filter(|l| !l.is_empty()) else { return "" };
    let mut pending = false;
    for c in list {
        // check runs use status/conclusion, commit statuses use state
        let status = c["status"].as_str().unwrap_or("");
        let concl = c["conclusion"].as_str().unwrap_or("");
        let state = c["state"].as_str().unwrap_or("");
        if matches!(concl, "FAILURE" | "TIMED_OUT" | "CANCELLED" | "STARTUP_FAILURE" | "ACTION_REQUIRED")
            || matches!(state, "FAILURE" | "ERROR")
        {
            return "fail";
        }
        if (!status.is_empty() && status != "COMPLETED") || matches!(state, "PENDING" | "EXPECTED") {
            pending = true;
        }
    }
    if pending {
        "pending"
    } else {
        "pass"
    }
}

pub fn parse(json: &str) -> Result<Vec<Item>> {
    let v: Value = serde_json::from_str(json).context("gh returned invalid JSON")?;
    let list = v.as_array().context("gh returned something other than a list")?;
    Ok(list
        .iter()
        .map(|i| Item {
            number: i["number"].as_u64().unwrap_or(0),
            title: i["title"].as_str().unwrap_or("").to_string(),
            author: i["author"]["login"].as_str().unwrap_or("").to_string(),
            url: i["url"].as_str().unwrap_or("").to_string(),
            state: i["state"].as_str().unwrap_or("").to_lowercase(),
            draft: i["isDraft"].as_bool().unwrap_or(false),
            branch: i["headRefName"].as_str().unwrap_or("").to_string(),
            checks: summarize_checks(&i["statusCheckRollup"]).to_string(),
            review: match i["reviewDecision"].as_str().unwrap_or("") {
                "APPROVED" => "approved",
                "CHANGES_REQUESTED" => "changes",
                "REVIEW_REQUIRED" => "review",
                _ => "",
            }
            .to_string(),
        })
        .collect())
}

/// Open PRs (`kind` = "pr") or issues (`kind` = "issue") of the repo, newest first.
pub fn list(repo: &Path, kind: &str) -> Result<Vec<Item>> {
    let (sub, fields) = match kind {
        "pr" => ("pr", "number,title,author,url,state,isDraft,headRefName,statusCheckRollup,reviewDecision"),
        "issue" => ("issue", "number,title,author,url,state"),
        _ => bail!("unknown kind {kind}"),
    };
    let o = git::quiet(&mut Command::new("gh"))
        .current_dir(repo)
        .env("GH_PROMPT_DISABLED", "1")
        .args([sub, "list", "--state", "open", "--limit", "40", "--json", fields])
        .output()
        .context("could not start gh. Install the GitHub CLI and run `gh auth login`.")?;
    if !o.status.success() {
        bail!("gh {sub} list failed: {}", String::from_utf8_lossy(&o.stderr).trim());
    }
    parse(&String::from_utf8_lossy(&o.stdout))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn checks_summary() {
        assert_eq!(summarize_checks(&json!([])), "");
        assert_eq!(summarize_checks(&json!([{"status":"COMPLETED","conclusion":"SUCCESS"}])), "pass");
        assert_eq!(
            summarize_checks(&json!([{"status":"COMPLETED","conclusion":"SUCCESS"},{"status":"IN_PROGRESS","conclusion":""}])),
            "pending"
        );
        assert_eq!(summarize_checks(&json!([{"status":"IN_PROGRESS"},{"status":"COMPLETED","conclusion":"FAILURE"}])), "fail");
        assert_eq!(summarize_checks(&json!([{"state":"SUCCESS"}])), "pass");
        assert_eq!(summarize_checks(&json!([{"state":"PENDING"}])), "pending");
    }

    #[test]
    fn parses_gh_output() {
        let j = r#"[{"number":7,"title":"Fix","author":{"login":"ana"},"url":"https://x/7","state":"OPEN","isDraft":true,"headRefName":"fix-it","statusCheckRollup":[{"status":"COMPLETED","conclusion":"SUCCESS"}],"reviewDecision":"APPROVED"}]"#;
        let v = parse(j).unwrap();
        assert_eq!(v[0].number, 7);
        assert_eq!((v[0].author.as_str(), v[0].state.as_str(), v[0].draft), ("ana", "open", true));
        assert_eq!((v[0].checks.as_str(), v[0].review.as_str(), v[0].branch.as_str()), ("pass", "approved", "fix-it"));
        assert!(parse("{}").is_err());
    }
}
