//! CI state of the pull request behind a worktree's branch, and merging it, through the GitHub CLI.
//! Every argv is fixed: nothing from the client reaches a command line, gh finds the PR from the branch checked out.

use anyhow::{bail, Result};
use axelstra_core::git;
use serde::Serialize;
use serde_json::Value;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

const READ_TIMEOUT: Duration = Duration::from_secs(15);
const MERGE_TIMEOUT: Duration = Duration::from_secs(45);

#[derive(Serialize, Debug, PartialEq, Clone)]
pub struct Check {
    pub name: String,
    /// "pass" | "fail" | "pending" | "skipped"
    pub state: String,
    pub link: String,
}

#[derive(Serialize, Debug, PartialEq, Default, Clone)]
pub struct Status {
    /// "" when a PR was read; "nopr" | "missing" (no gh) | "auth" (not logged in) | "error" otherwise
    pub problem: String,
    pub detail: String,
    pub number: u64,
    pub title: String,
    pub url: String,
    /// "open" | "merged" | "closed"
    pub pr_state: String,
    pub draft: bool,
    /// "pass" | "fail" | "pending" | "none"
    pub ci: String,
    pub checks: Vec<Check>,
    /// "approved" | "changes" | "review" | ""
    pub review: String,
    /// "yes" | "no" | "unknown"
    pub mergeable: String,
    /// gh's mergeStateStatus, lowercased (clean, blocked, behind, dirty, ...)
    pub merge_state: String,
    pub can_merge: bool,
}

/// One check run or commit status as one of pass, fail, pending, skipped.
fn classify(status: &str, conclusion: &str, state: &str) -> &'static str {
    let (status, conclusion, state) = (status.to_uppercase(), conclusion.to_uppercase(), state.to_uppercase());
    if matches!(conclusion.as_str(), "FAILURE" | "TIMED_OUT" | "CANCELLED" | "STARTUP_FAILURE" | "ACTION_REQUIRED")
        || matches!(state.as_str(), "FAILURE" | "ERROR" | "CANCELLED" | "TIMED_OUT" | "ACTION_REQUIRED" | "STARTUP_FAILURE")
    {
        return "fail";
    }
    if matches!(conclusion.as_str(), "SKIPPED" | "NEUTRAL") || matches!(state.as_str(), "SKIPPED" | "NEUTRAL") {
        return "skipped";
    }
    if (!status.is_empty() && status != "COMPLETED")
        || matches!(state.as_str(), "PENDING" | "EXPECTED" | "QUEUED" | "IN_PROGRESS" | "WAITING")
    {
        return "pending";
    }
    "pass"
}

/// Checks from `gh pr checks --json name,state,link,bucket`.
pub fn parse_checks(json: &str) -> Result<Vec<Check>> {
    let v: Value = serde_json::from_str(json)?;
    let list = v.as_array().ok_or_else(|| anyhow::anyhow!("gh returned something other than a list"))?;
    Ok(list
        .iter()
        .map(|c| {
            let state = match c["bucket"].as_str().unwrap_or("") {
                "pass" => "pass",
                "fail" | "cancel" => "fail",
                "pending" => "pending",
                "skipping" => "skipped",
                _ => classify("", "", c["state"].as_str().unwrap_or("")),
            };
            Check {
                name: c["name"].as_str().unwrap_or("").to_string(),
                state: state.to_string(),
                link: c["link"].as_str().unwrap_or("").to_string(),
            }
        })
        .collect())
}

/// Checks from the `statusCheckRollup` of `gh pr view`; used when `gh pr checks` has nothing usable.
pub fn rollup_checks(rollup: &Value) -> Vec<Check> {
    let Some(list) = rollup.as_array() else { return vec![] };
    list.iter()
        .map(|c| {
            let s = |k: &str| c[k].as_str().unwrap_or("");
            let name = if s("name").is_empty() { s("context") } else { s("name") };
            let link = if s("detailsUrl").is_empty() { s("targetUrl") } else { s("detailsUrl") };
            Check { name: name.to_string(), state: classify(s("status"), s("conclusion"), s("state")).to_string(), link: link.to_string() }
        })
        .collect()
}

/// One word for the whole list: a failure wins, then anything running; no checks at all is "none".
pub fn overall(checks: &[Check]) -> &'static str {
    if checks.is_empty() {
        "none"
    } else if checks.iter().any(|c| c.state == "fail") {
        "fail"
    } else if checks.iter().any(|c| c.state == "pending") {
        "pending"
    } else {
        "pass"
    }
}

/// Fills a Status from `gh pr view --json number,title,url,state,isDraft,statusCheckRollup,reviewDecision,mergeable,mergeStateStatus`.
/// `checks` is the list from `gh pr checks` when that worked.
pub fn parse_view(json: &str, checks: Option<Vec<Check>>) -> Result<Status> {
    let v: Value = serde_json::from_str(json)?;
    if !v.is_object() {
        bail!("gh returned something other than an object");
    }
    let s = |k: &str| v[k].as_str().unwrap_or("");
    let checks = match checks {
        Some(c) if !c.is_empty() => c,
        _ => rollup_checks(&v["statusCheckRollup"]),
    };
    let pr_state = match s("state") {
        "MERGED" => "merged",
        "CLOSED" => "closed",
        _ => "open",
    };
    let mergeable = match s("mergeable") {
        "MERGEABLE" => "yes",
        "CONFLICTING" => "no",
        _ => "unknown",
    };
    let merge_state = s("mergeStateStatus").to_lowercase();
    let ci = overall(&checks);
    let draft = v["isDraft"].as_bool().unwrap_or(false);
    let can_merge = pr_state == "open"
        && !draft
        && mergeable == "yes"
        && matches!(ci, "pass" | "none")
        && matches!(merge_state.as_str(), "clean" | "has_hooks" | "");
    Ok(Status {
        problem: String::new(),
        detail: String::new(),
        number: v["number"].as_u64().unwrap_or(0),
        title: s("title").to_string(),
        url: s("url").to_string(),
        pr_state: pr_state.to_string(),
        draft,
        ci: ci.to_string(),
        checks,
        review: match s("reviewDecision") {
            "APPROVED" => "approved",
            "CHANGES_REQUESTED" => "changes",
            "REVIEW_REQUIRED" => "review",
            _ => "",
        }
        .to_string(),
        mergeable: mergeable.to_string(),
        merge_state,
        can_merge,
    })
}

/// What a failed gh run means for the user: no PR on this branch, not logged in, or something else.
pub fn classify_failure(stderr: &str) -> &'static str {
    let e = stderr.to_lowercase();
    if e.contains("no pull requests found") || e.contains("no open pull requests") {
        "nopr"
    } else if e.contains("gh auth login") || e.contains("not logged in") || e.contains("authentication") || e.contains("gh_token") {
        "auth"
    } else {
        "error"
    }
}

struct Out {
    ok: bool,
    stdout: String,
    stderr: String,
}

enum RunError {
    Missing,
    Timeout,
    Other(String),
}

/// Most of each stream kept from one gh call.
const GH_MAX_OUT: usize = 4 << 20;

/// Runs gh with a fixed argv in `cwd`, killed (with its children) after `limit`. Never prompts.
fn gh(cwd: &Path, args: &[&str], limit: Duration) -> Result<Out, RunError> {
    let mut cmd = Command::new("gh");
    cmd.current_dir(cwd).env("GH_PROMPT_DISABLED", "1").env("NO_COLOR", "1").args(args);
    let o = axelstra_core::exec::run_capped(&mut cmd, limit, GH_MAX_OUT).map_err(|e| match e.downcast_ref::<std::io::Error>() {
        Some(io) if io.kind() == std::io::ErrorKind::NotFound => RunError::Missing,
        _ => RunError::Other(e.to_string()),
    })?;
    if o.timed_out {
        return Err(RunError::Timeout);
    }
    Ok(Out { ok: o.success(), stdout: o.stdout_lossy(), stderr: o.stderr_lossy() })
}

fn problem(kind: &str, detail: impl Into<String>) -> Status {
    Status { problem: kind.to_string(), detail: detail.into(), ci: "none".into(), mergeable: "unknown".into(), ..Status::default() }
}

const VIEW_FIELDS: &str = "number,title,url,state,isDraft,statusCheckRollup,reviewDecision,mergeable,mergeStateStatus";

/// State of the PR for the branch checked out in `wt`. A missing PR, a missing gh or a missing login are answers
/// (the `problem` field), not errors: the panel shows each with its own hint.
pub fn status(wt: &Path) -> Status {
    let view = match gh(wt, &["pr", "view", "--json", VIEW_FIELDS], READ_TIMEOUT) {
        Ok(o) => o,
        Err(RunError::Missing) => return problem("missing", "gh not found"),
        Err(RunError::Timeout) => return problem("error", "gh took too long"),
        Err(RunError::Other(e)) => return problem("error", e),
    };
    if !view.ok {
        return problem(classify_failure(&view.stderr), view.stderr.trim());
    }
    // `gh pr checks` exits non-zero while checks run or fail but still prints the JSON; "no checks" prints none
    let checks =
        gh(wt, &["pr", "checks", "--json", "name,state,link,bucket"], READ_TIMEOUT).ok().and_then(|o| parse_checks(&o.stdout).ok());
    match parse_view(&view.stdout, checks) {
        Ok(s) => s,
        Err(e) => problem("error", e.to_string()),
    }
}

/// A branch name safe to hand to `git push origin --delete`: no leading dash, no `..`, no ref syntax characters,
/// and never the trunk.
pub fn deletable_branch(b: &str) -> bool {
    !b.is_empty()
        && b.len() <= 200
        && !matches!(b, "main" | "master" | "HEAD")
        && !b.starts_with(['-', '/', '.'])
        && !b.ends_with(['/', '.'])
        && !b.contains("..")
        && !b.contains("//")
        && !b.ends_with(".lock")
        && b.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '/'))
}

/// Argv for `gh pr merge`. `--delete-branch` is never part of it: gh would try to delete the local branch, which is
/// checked out in this worktree, and fail after the merge went through.
fn merge_args(auto: bool) -> Vec<&'static str> {
    let mut args = vec!["pr", "merge", "--squash"];
    if auto {
        args.push("--auto");
    }
    args
}

/// Argv that deletes the remote branch, or None when the name is not safe to pass on.
fn delete_remote_args(branch: &str) -> Option<Vec<&str>> {
    deletable_branch(branch).then(|| vec!["push", "origin", "--delete", branch])
}

/// Squash-merges the PR (or arms auto-merge). The same gate as the button: a direct merge only when `status` says
/// it can be merged, so a stale click cannot merge a failing PR. With `delete_branch` the remote branch is deleted
/// after a direct merge; the local branch and the worktree are left for the janitor.
pub fn merge(wt: &Path, auto: bool, delete_branch: bool) -> Result<String> {
    let st = status(wt);
    if !st.problem.is_empty() {
        bail!("no pull request to merge: {}", st.detail);
    }
    if st.pr_state != "open" {
        bail!("pull request #{} is already {}", st.number, st.pr_state);
    }
    if !auto && !st.can_merge {
        bail!("pull request #{} cannot be merged yet (checks {}, mergeable {}, state {})", st.number, st.ci, st.mergeable, st.merge_state);
    }
    let o = gh(wt, &merge_args(auto), MERGE_TIMEOUT).map_err(|e| match e {
        RunError::Missing => anyhow::anyhow!("could not start gh. Install the GitHub CLI and run `gh auth login`."),
        RunError::Timeout => anyhow::anyhow!("gh pr merge took too long"),
        RunError::Other(e) => anyhow::anyhow!(e),
    })?;
    if !o.ok {
        bail!("gh pr merge failed: {}", o.stderr.trim());
    }
    let mut out = o.stdout.trim().to_string();
    // armed auto-merge has not merged anything yet: the branch must stay until it does
    if delete_branch && !auto {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&delete_remote(wt));
    }
    Ok(out)
}

/// Deletes the remote branch of `wt` after a merge and says in pt-PT what happened. Never fails the merge.
fn delete_remote(wt: &Path) -> String {
    let branch = git::current_branch(wt).unwrap_or_default();
    let Some(args) = delete_remote_args(&branch) else {
        return "Merge feito. Não apaguei o ramo remoto: o nome do ramo não é seguro para apagar.".into();
    };
    match git::run(wt, &args) {
        Ok(o) if o.ok => format!("Merge feito e ramo remoto {branch} apagado. O ramo local e o worktree ficam para o janitor."),
        Ok(o) => format!(
            "Merge feito, mas não consegui apagar o ramo remoto {branch}: {}. O ramo local e o worktree ficam para o janitor.",
            o.stderr.lines().last().unwrap_or("erro desconhecido")
        ),
        Err(e) => format!("Merge feito, mas não consegui apagar o ramo remoto {branch}: {e}."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const CHECKS: &str = r#"[
      {"name":"build","state":"SUCCESS","link":"https://x/1","bucket":"pass"},
      {"name":"lint","state":"IN_PROGRESS","link":"https://x/2","bucket":"pending"},
      {"name":"docs","state":"SKIPPED","link":"","bucket":"skipping"},
      {"name":"e2e","state":"FAILURE","link":"https://x/3","bucket":"fail"}]"#;

    const VIEW: &str = r#"{"number":12,"title":"Add thing","url":"https://github.com/o/r/pull/12","state":"OPEN","isDraft":false,
      "reviewDecision":"APPROVED","mergeable":"MERGEABLE","mergeStateStatus":"CLEAN",
      "statusCheckRollup":[{"name":"build","status":"COMPLETED","conclusion":"SUCCESS","detailsUrl":"https://x/1"},
                           {"context":"ci/legacy","state":"SUCCESS","targetUrl":"https://x/9"}]}"#;

    #[test]
    fn parses_checks_by_bucket() {
        let c = parse_checks(CHECKS).unwrap();
        let states: Vec<_> = c.iter().map(|c| c.state.as_str()).collect();
        assert_eq!(states, ["pass", "pending", "skipped", "fail"]);
        assert_eq!(c[0], Check { name: "build".into(), state: "pass".into(), link: "https://x/1".into() });
        assert_eq!(overall(&c), "fail");
        assert!(parse_checks("{}").is_err());
        assert!(parse_checks("").is_err());
    }

    #[test]
    fn bucketless_checks_fall_back_to_state() {
        let c = parse_checks(r#"[{"name":"a","state":"QUEUED"},{"name":"b","state":"CANCELLED"},{"name":"c","state":"NEUTRAL"}]"#).unwrap();
        let states: Vec<_> = c.iter().map(|c| c.state.as_str()).collect();
        assert_eq!(states, ["pending", "fail", "skipped"]);
    }

    #[test]
    fn overall_ranks_fail_then_pending() {
        let mk = |s: &[&str]| s.iter().map(|x| Check { name: "n".into(), state: x.to_string(), link: String::new() }).collect::<Vec<_>>();
        assert_eq!(overall(&[]), "none");
        assert_eq!(overall(&mk(&["pass", "skipped"])), "pass");
        assert_eq!(overall(&mk(&["pass", "pending"])), "pending");
        assert_eq!(overall(&mk(&["pending", "fail"])), "fail");
        assert_eq!(overall(&mk(&["skipped"])), "pass");
    }

    #[test]
    fn rollup_mixes_check_runs_and_statuses() {
        let r = rollup_checks(&json!([
            {"name":"build","status":"COMPLETED","conclusion":"FAILURE","detailsUrl":"d"},
            {"context":"ci/x","state":"PENDING","targetUrl":"t"},
            {"name":"w","status":"IN_PROGRESS","conclusion":""}]));
        assert_eq!((r[0].name.as_str(), r[0].state.as_str(), r[0].link.as_str()), ("build", "fail", "d"));
        assert_eq!((r[1].name.as_str(), r[1].state.as_str(), r[1].link.as_str()), ("ci/x", "pending", "t"));
        assert_eq!(r[2].state, "pending");
        assert!(rollup_checks(&json!(null)).is_empty());
    }

    #[test]
    fn view_with_passing_rollup_can_merge() {
        let s = parse_view(VIEW, None).unwrap();
        assert_eq!((s.number, s.pr_state.as_str(), s.ci.as_str(), s.review.as_str()), (12, "open", "pass", "approved"));
        assert_eq!((s.mergeable.as_str(), s.merge_state.as_str()), ("yes", "clean"));
        assert_eq!(s.checks.len(), 2);
        assert!(s.can_merge);
    }

    #[test]
    fn checks_list_wins_over_rollup_when_present() {
        let s = parse_view(VIEW, Some(parse_checks(CHECKS).unwrap())).unwrap();
        assert_eq!(s.ci, "fail");
        assert_eq!(s.checks.len(), 4);
        assert!(!s.can_merge);
        // an empty list from gh pr checks falls back to the rollup
        assert_eq!(parse_view(VIEW, Some(vec![])).unwrap().checks.len(), 2);
    }

    #[test]
    fn merge_gate() {
        let with = |patch: &str| {
            let mut v: Value = serde_json::from_str(VIEW).unwrap();
            for (k, val) in serde_json::from_str::<Value>(patch).unwrap().as_object().unwrap() {
                v[k] = val.clone();
            }
            parse_view(&v.to_string(), None).unwrap()
        };
        assert!(!with(r#"{"isDraft":true}"#).can_merge);
        assert!(!with(r#"{"mergeable":"CONFLICTING","mergeStateStatus":"DIRTY"}"#).can_merge);
        assert!(!with(r#"{"mergeStateStatus":"BLOCKED"}"#).can_merge);
        assert!(!with(r#"{"mergeStateStatus":"BEHIND"}"#).can_merge);
        assert!(!with(r#"{"mergeable":"UNKNOWN"}"#).can_merge);
        let merged = with(r#"{"state":"MERGED"}"#);
        assert_eq!(merged.pr_state, "merged");
        assert!(!merged.can_merge);
        // a repo without CI can still be merged
        assert!(with(r#"{"statusCheckRollup":[]}"#).can_merge);
        assert_eq!(with(r#"{"statusCheckRollup":[]}"#).ci, "none");
        assert_eq!(with(r#"{"reviewDecision":"CHANGES_REQUESTED"}"#).review, "changes");
    }

    #[test]
    fn failures_are_told_apart() {
        assert_eq!(classify_failure("no pull requests found for branch \"x\""), "nopr");
        assert_eq!(classify_failure("To get started with GitHub CLI, please run:  gh auth login"), "auth");
        assert_eq!(classify_failure("You are not logged in to any GitHub hosts."), "auth");
        assert_eq!(classify_failure("HTTP 502"), "error");
    }

    #[test]
    fn merge_never_asks_gh_to_delete_the_branch() {
        assert_eq!(merge_args(false), ["pr", "merge", "--squash"]);
        assert_eq!(merge_args(true), ["pr", "merge", "--squash", "--auto"]);
    }

    #[test]
    fn remote_delete_argv_is_fixed_and_the_ref_validated() {
        assert_eq!(delete_remote_args("ax/polish").unwrap(), ["push", "origin", "--delete", "ax/polish"]);
        for bad in ["", "main", "master", "-f", "--force", "a..b", "a b", "a;rm", "x/", "/x", "a//b", "a.lock", "feat~1", "a:b", "$(x)"] {
            assert!(delete_remote_args(bad).is_none(), "{bad:?}");
        }
    }

    #[test]
    fn bad_view_json_is_an_error() {
        assert!(parse_view("[]", None).is_err());
        assert!(parse_view("nope", None).is_err());
    }
}
