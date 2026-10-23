//! `axelstra statusline-tee`: Claude Code's status line command for sessions started by Axelstra. It saves the
//! JSON Claude passes in (context window, 5 h and weekly limits...) where the app reads it, then runs the
//! status line the user had configured themselves, so what they see in the terminal does not change.

use anyhow::Result;
use serde_json::Value;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};

fn home() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from)
}

/// The user's own `statusLine.command`: project local, project shared, then user settings (Claude's order).
fn user_command() -> Option<String> {
    let mut files = vec![];
    if let Ok(cwd) = std::env::current_dir() {
        files.push(cwd.join(".claude").join("settings.local.json"));
        files.push(cwd.join(".claude").join("settings.json"));
    }
    if let Some(h) = home() {
        files.push(h.join(".claude").join("settings.json"));
    }
    files.into_iter().find_map(|f| {
        let v: Value = serde_json::from_str(&std::fs::read_to_string(f).ok()?).ok()?;
        v["statusLine"]["command"].as_str().map(String::from)
    })
}

fn save(input: &str) {
    let (Ok(key), Ok(dir)) = (std::env::var("AXELSTRA_KEY"), std::env::var("AXELSTRA_DIR")) else { return };
    if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return;
    }
    let dir = PathBuf::from(dir).join("status");
    let _ = std::fs::create_dir_all(&dir);
    // written aside and renamed, so the app never reads half a file
    let tmp = dir.join(format!("{key}.tmp"));
    if std::fs::write(&tmp, input).is_ok() {
        let _ = std::fs::rename(&tmp, dir.join(format!("{key}.json")));
    }
}

fn run_user(cmdline: &str, input: &str) {
    let spawn = |program: &str, args: &[&str]| Command::new(program).args(args).arg(cmdline).stdin(Stdio::piped()).spawn();
    let child = if cfg!(windows) {
        // Claude runs status lines through a POSIX shell (Git Bash on Windows); fall back to cmd
        // not plain `bash`: on Windows that is often the WSL stub
        let git = [r"C:\Program Files\Git\bin\bash.exe", r"C:\Program Files (x86)\Git\bin\bash.exe"]
            .into_iter()
            .find(|p| std::path::Path::new(p).exists());
        match git {
            Some(bash) => spawn(bash, &["-c"]),
            None => spawn("cmd", &["/d", "/c"]),
        }
    } else {
        spawn("sh", &["-c"])
    };
    if let Ok(mut child) = child {
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(input.as_bytes());
        }
        let _ = child.wait();
    }
}

pub fn tee() -> Result<()> {
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input)?;
    save(&input);
    if let Some(cmdline) = user_command() {
        run_user(&cmdline, &input);
    }
    Ok(())
}
