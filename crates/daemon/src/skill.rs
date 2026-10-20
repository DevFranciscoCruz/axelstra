//! The `/axelstration` skill, shipped inside the binary and installed by the app. Like other agent tools it is a
//! thin discovery stub (`SKILL.md`); the real guide is served by `axelstra skills get axelstration`, so the
//! instructions always match the installed binary. The stub goes to the shared `~/.agents/skills` (read by
//! Codex and others) and to `~/.claude/skills` (Claude Code).
use anyhow::{anyhow, Result};
use std::path::PathBuf;

const STUB: &str = include_str!("../../../skills/axelstration/SKILL.md");
const GUIDE: &str = include_str!("../../../skills/axelstration/GUIDE.md");

/// The guide text printed by `axelstra skills get axelstration`.
pub fn guide() -> &'static str {
    GUIDE
}

fn home() -> Result<PathBuf> {
    std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from).ok_or_else(|| anyhow!("no home directory"))
}

/// (skills folder, folders whose existence says the user has an agent CLI that reads it)
fn targets(home: &std::path::Path) -> Vec<(PathBuf, Vec<PathBuf>)> {
    vec![
        (home.join(".claude").join("skills"), vec![home.join(".claude")]),
        (home.join(".agents").join("skills"), vec![home.join(".agents"), home.join(".codex")]),
    ]
}

fn write(skills: PathBuf) -> Result<PathBuf> {
    let file = skills.join("axelstration").join("SKILL.md");
    std::fs::create_dir_all(file.parent().unwrap())?;
    std::fs::write(&file, STUB)?;
    Ok(file)
}

/// Writes the stub for every agent CLI the user has when missing or outdated; returns how many files it wrote.
pub fn install() -> Result<usize> {
    let home = home()?;
    let mut n = 0;
    for (skills, markers) in targets(&home) {
        if !markers.iter().any(|m| m.exists()) {
            continue;
        }
        let file = skills.join("axelstration").join("SKILL.md");
        if std::fs::read_to_string(&file).map(|t| t == STUB).unwrap_or(false) {
            continue;
        }
        write(skills)?;
        n += 1;
    }
    Ok(n)
}

/// Explicit install (`axelstra install-skill`): every location, creating folders when needed.
pub fn install_forced() -> Result<Vec<PathBuf>> {
    targets(&home()?).into_iter().map(|(skills, _)| write(skills)).collect()
}
