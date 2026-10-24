//! `git.timeline`: what happened in one worktree, oldest to newest or the other way round: the commits of its branch,
//! merges (into it and of it into the default branch), the reports its agents left, and, as totals, the tokens spent
//! and the CI state of its pull request. The app shows it as a panel and exports it as markdown.

use crate::agentlog::WorktreeUsage;
use crate::checks::Status;
use crate::notes::NoteInfo;
use anyhow::Result;
use axelstra_core::git;
use serde::Serialize;
use std::path::Path;

/// Most commits listed: a branch far ahead of the trunk would otherwise make the answer huge.
const MAX_COMMITS: usize = 300;

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct Event {
    /// seconds since the epoch
    pub at: i64,
    /// "commit" | "merge" | "integrated" | "report"
    pub kind: String,
    pub title: String,
    pub detail: String,
}

#[derive(Serialize, Debug, Clone, Default)]
pub struct Timeline {
    pub branch: String,
    pub base: String,
    /// newest first
    pub events: Vec<Event>,
    pub tokens: Option<WorktreeUsage>,
    pub ci: Option<Status>,
}

const SEP: char = '\u{1f}';

/// Lines of `git log --format=%H<US>%at<US>%an<US>%P<US>%s`. A commit with two parents is a merge.
pub fn parse_log(text: &str) -> Vec<Event> {
    text.lines()
        .filter_map(|l| {
            let mut f = l.splitn(5, SEP);
            let (sha, at, author, parents, subject) = (f.next()?, f.next()?, f.next()?, f.next()?, f.next()?);
            let merge = parents.split_whitespace().count() > 1;
            Some(Event {
                at: at.trim().parse().ok()?,
                kind: if merge { "merge" } else { "commit" }.into(),
                title: subject.trim().to_string(),
                detail: format!("{} · {}", sha.chars().take(7).collect::<String>(), author.trim()),
            })
        })
        .collect()
}

fn log(wt: &Path, range: &str, extra: &[&str]) -> Vec<Event> {
    let fmt = format!("--format=%H{SEP}%at{SEP}%an{SEP}%P{SEP}%s");
    let max = format!("--max-count={MAX_COMMITS}");
    let mut args = vec!["log", fmt.as_str(), max.as_str()];
    args.extend_from_slice(extra);
    args.push(range);
    match git::run(wt, &args) {
        Ok(o) if o.ok => parse_log(&o.stdout),
        _ => vec![],
    }
}

/// When the branch of `wt` was merged into `base`: the merge commit on `base` that brought its tip in, if there is one.
fn merged_into_base(wt: &Path, base: &str) -> Option<(Event, String)> {
    let contained = git::run(wt, &["merge-base", "--is-ancestor", "HEAD", base]).ok()?.ok;
    if !contained {
        return None;
    }
    let range = format!("HEAD..{base}");
    // oldest first: the first merge on the path from the tip to the trunk is the one that integrated it
    let o = git::run(wt, &["log", "--ancestry-path", "--merges", "--reverse", "--format=%H", &range]).ok().filter(|o| o.ok)?;
    let sha = o.stdout.lines().next()?.trim().to_string();
    let e = log(wt, &sha, &["--max-count=1"]).into_iter().next()?;
    Some((
        Event { kind: "integrated".into(), title: format!("Integrado em {base}"), detail: format!("{} · {}", e.detail, e.title), ..e },
        sha,
    ))
}

/// Pure assembly, so it can be tested without git: commits, merges and reports in one list, newest first.
pub fn assemble(mut events: Vec<Event>, notes: &[NoteInfo]) -> Vec<Event> {
    for n in notes {
        events.push(Event {
            at: n.modified as i64,
            kind: "report".into(),
            title: n.name.clone(),
            detail: if n.excerpt.is_empty() { n.status.clone() } else { format!("{} · {}", n.status, n.excerpt) },
        });
    }
    events.sort_by(|a, b| b.at.cmp(&a.at).then_with(|| a.title.cmp(&b.title)));
    events
}

/// The timeline of `wt`. `notes` are its task files; `ci` is only asked for when the caller wants it (it runs `gh`).
pub fn build(wt: &Path, notes: &[NoteInfo], with_ci: bool) -> Result<Timeline> {
    let repo_base = git::default_branch(wt);
    let branch = git::current_branch(wt).unwrap_or_default();
    let mut events = vec![];
    if branch != repo_base {
        // once merged, the trunk contains the branch: its commits are those since the merge's first parent
        let merged = merged_into_base(wt, &repo_base);
        let range = match &merged {
            Some((_, sha)) => format!("{sha}^1..HEAD"),
            None => format!("{repo_base}..HEAD"),
        };
        events = log(wt, &range, &[]);
        events.extend(merged.map(|m| m.0));
    }
    let events = assemble(events, notes);
    let tokens = crate::agentlog::worktree_usage(&[wt.to_string_lossy().into_owned()])
        .into_iter()
        .next()
        .filter(|u| u.input + u.output + u.cache_read + u.cache_write > 0);
    let ci = if with_ci { Some(crate::checks::status(wt)).filter(|s| s.problem.is_empty()) } else { None };
    Ok(Timeline { branch, base: repo_base, events, tokens, ci })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(at: i64, kind: &str, title: &str) -> Event {
        Event { at, kind: kind.into(), title: title.into(), detail: String::new() }
    }

    #[test]
    fn log_lines_become_commits_and_merges() {
        let text = format!("abcdef123{SEP}100{SEP}Ana{SEP}p1{SEP}fix: a\nfff0000111{SEP}200{SEP}Rui{SEP}p1 p2{SEP}Merge x\nbroken line");
        let evs = parse_log(&text);
        assert_eq!(evs.len(), 2);
        assert_eq!((evs[0].kind.as_str(), evs[0].at, evs[0].title.as_str()), ("commit", 100, "fix: a"));
        assert_eq!(evs[0].detail, "abcdef1 · Ana");
        assert_eq!(evs[1].kind, "merge");
    }

    #[test]
    fn reports_are_mixed_in_by_time_newest_first() {
        let notes = vec![NoteInfo { name: "Tarefa 1".into(), status: "ok".into(), excerpt: "feito".into(), modified: 150 }];
        let out = assemble(vec![ev(100, "commit", "a"), ev(200, "commit", "b")], &notes);
        let kinds: Vec<_> = out.iter().map(|e| (e.at, e.kind.as_str())).collect();
        assert_eq!(kinds, vec![(200, "commit"), (150, "report"), (100, "commit")]);
        assert_eq!(out[1].detail, "ok · feito");
    }

    fn git_in(r: &Path, args: &[&str]) {
        let mut a = vec!["-c", "user.name=t", "-c", "user.email=t@t"];
        a.extend_from_slice(args);
        git::must(r, &a).unwrap();
    }

    #[test]
    fn a_branch_lists_its_commits_and_when_it_was_merged() {
        let r = std::env::temp_dir().join(format!("axl-timeline-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&r);
        std::fs::create_dir_all(&r).unwrap();
        git::must(&r, &["init", "-q", "-b", "main"]).unwrap();
        std::fs::write(r.join("a.txt"), "1").unwrap();
        git_in(&r, &["add", "."]);
        git_in(&r, &["commit", "-qm", "init"]);
        git_in(&r, &["checkout", "-qb", "topic"]);
        std::fs::write(r.join("b.txt"), "2").unwrap();
        git_in(&r, &["add", "."]);
        git_in(&r, &["commit", "-qm", "add b"]);
        let t = build(&r, &[], false).unwrap();
        assert_eq!((t.branch.as_str(), t.base.as_str()), ("topic", "main"));
        assert_eq!(t.events.iter().map(|e| e.title.as_str()).collect::<Vec<_>>(), vec!["add b"]);
        // not merged yet
        assert!(t.events.iter().all(|e| e.kind != "integrated"));
        // merge with a merge commit
        git_in(&r, &["checkout", "-q", "main"]);
        std::fs::write(r.join("c.txt"), "3").unwrap();
        git_in(&r, &["add", "."]);
        git_in(&r, &["commit", "-qm", "trunk work"]);
        git_in(&r, &["merge", "--no-ff", "-qm", "Merge topic", "topic"]);
        git_in(&r, &["checkout", "-q", "topic"]);
        let t = build(&r, &[], false).unwrap();
        assert!(t.events.iter().any(|e| e.kind == "integrated" && e.detail.contains("Merge topic")), "{:?}", t.events);
        assert!(t.events.iter().any(|e| e.title == "add b"), "the merged branch still lists its commits");
        assert!(t.tokens.is_none() && t.ci.is_none());
        let _ = std::fs::remove_dir_all(&r);
    }
}
