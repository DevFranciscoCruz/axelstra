//! The `/axelstration` skill for Claude Code, shipped inside the binary so the app installs it by itself.
use anyhow::{anyhow, Result};
use std::path::PathBuf;

const SKILL: &str = include_str!("../../../skills/axelstration/SKILL.md");

fn target() -> Result<PathBuf> {
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).ok_or_else(|| anyhow!("no home directory"))?;
    Ok(PathBuf::from(home).join(".claude").join("skills").join("axelstration").join("SKILL.md"))
}

/// Writes the skill when it is missing or differs from this build's copy; returns whether it wrote.
/// Skipped when the user has no Claude Code folder at all (nothing would read it).
pub fn install() -> Result<bool> {
    let file = target()?;
    let claude = file.ancestors().nth(3).map(|p| p.to_path_buf()).ok_or_else(|| anyhow!("bad path"))?;
    if !claude.exists() {
        return Ok(false);
    }
    if std::fs::read_to_string(&file).map(|t| t == SKILL).unwrap_or(false) {
        return Ok(false);
    }
    std::fs::create_dir_all(file.parent().unwrap())?;
    std::fs::write(&file, SKILL)?;
    Ok(true)
}

/// Explicit install (`axelstra install-skill`): creates ~/.claude when needed.
pub fn install_forced() -> Result<PathBuf> {
    let file = target()?;
    std::fs::create_dir_all(file.parent().unwrap())?;
    std::fs::write(&file, SKILL)?;
    Ok(file)
}
