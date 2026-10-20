//! The `/axelstration` skill, shipped inside the binary so the app installs it by itself for every
//! agent CLI that reads `SKILL.md` folders (Claude Code: ~/.claude/skills, Codex: ~/.codex/skills).
use anyhow::{anyhow, Result};
use std::path::PathBuf;

const SKILL: &str = include_str!("../../../skills/axelstration/SKILL.md");
/// config folder of each supported agent CLI, relative to the home directory
const AGENT_DIRS: [&str; 2] = [".claude", ".codex"];

fn home() -> Result<PathBuf> {
    std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from).ok_or_else(|| anyhow!("no home directory"))
}

fn write(root: PathBuf) -> Result<PathBuf> {
    let file = root.join("skills").join("axelstration").join("SKILL.md");
    std::fs::create_dir_all(file.parent().unwrap())?;
    std::fs::write(&file, SKILL)?;
    Ok(file)
}

/// Writes the skill for every agent CLI the user has (its config folder exists) when missing or outdated;
/// returns how many files were written.
pub fn install() -> Result<usize> {
    let home = home()?;
    let mut n = 0;
    for d in AGENT_DIRS {
        let root = home.join(d);
        if !root.exists() {
            continue;
        }
        let file = root.join("skills").join("axelstration").join("SKILL.md");
        if std::fs::read_to_string(&file).map(|t| t == SKILL).unwrap_or(false) {
            continue;
        }
        write(root)?;
        n += 1;
    }
    Ok(n)
}

/// Explicit install (`axelstra install-skill`): every supported CLI, creating its folder when needed.
pub fn install_forced() -> Result<Vec<PathBuf>> {
    let home = home()?;
    AGENT_DIRS.iter().map(|d| write(home.join(d))).collect()
}
