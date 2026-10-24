//! `env.tools`: which of the command line tools Axelstra needs are installed, and their versions.

use axelstra_core::{agents, exec};
use serde::Serialize;
use std::process::Command;
use std::time::Duration;

/// Tool id -> executable names looked up on PATH. `git` is required; the agent CLI and `gh` only enable features.
const TOOLS: [(&str, &[&str]); 3] = [("git", &["git"]), ("claude", &["claude"]), ("gh", &["gh"])];

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Tool {
    pub id: &'static str,
    pub found: bool,
    pub path: Option<String>,
    pub version: Option<String>,
}

/// The first dotted version number in the first line of a `--version` output ("git version 2.45.1.windows.1" ->
/// "2.45.1.windows.1", "2.1.0 (Claude Code)" -> "2.1.0").
pub fn parse_version(out: &str) -> Option<String> {
    let line = out.lines().next()?;
    line.split_whitespace()
        .find(|t| t.starts_with(|c: char| c.is_ascii_digit()) && t.contains('.'))
        .map(|t| t.trim_matches(|c: char| !c.is_ascii_alphanumeric()).to_string())
}

fn version_of(exe: &std::path::Path) -> Option<String> {
    let out = exec::run_capped(Command::new(exe).arg("--version"), Duration::from_secs(5), 4096).ok()?;
    if !out.success() {
        return None;
    }
    parse_version(&out.stdout_lossy())
}

pub fn detect() -> Vec<Tool> {
    let path_var = std::env::var_os("PATH").unwrap_or_default();
    TOOLS
        .iter()
        .map(|(id, exes)| {
            let found = agents::find_in(exes, &path_var);
            Tool {
                id,
                found: found.is_some(),
                version: found.as_deref().and_then(version_of),
                path: found.map(|p| p.to_string_lossy().into_owned()),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_are_read_from_the_usual_outputs() {
        assert_eq!(parse_version("git version 2.45.1.windows.1\n").as_deref(), Some("2.45.1.windows.1"));
        assert_eq!(parse_version("2.1.0 (Claude Code)").as_deref(), Some("2.1.0"));
        assert_eq!(parse_version("gh version 2.50.0 (2024-06-05)\nhttps://github.com/cli/cli").as_deref(), Some("2.50.0"));
        assert_eq!(parse_version("no numbers here"), None);
        assert_eq!(parse_version(""), None);
    }

    #[test]
    fn detect_reports_every_tool_once() {
        let t = detect();
        assert_eq!(t.iter().map(|t| t.id).collect::<Vec<_>>(), ["git", "claude", "gh"]);
        assert!(t.iter().all(|t| t.found == t.path.is_some()));
    }
}
