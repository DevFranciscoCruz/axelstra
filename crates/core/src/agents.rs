//! The one table of coding agents Axelstra can start: how to find them, how to launch them interactively and
//! unattended, and whether they resume. Everything is array argv; there is no shell string in here.
//!
//! Flags checked against the installed CLI's own `--help` on this machine: claude 2.1.309 and codex-cli 0.154.0.
//! UNVERIFIED (CLI not installed here, taken from the official docs): every gemini, opencode and aider flag.
//! `codex --full-auto` no longer exists in 0.154, so codex uses `--sandbox workspace-write` instead.

use anyhow::{bail, Result};
use std::path::{Path, PathBuf};

/// Dangerous means: skips the agent's permission prompts. Never added unless the caller asks for it.
#[derive(Debug)]
pub struct Agent {
    pub id: &'static str,
    pub name: &'static str,
    /// old ids that still mean this agent (cards saved with "chatgpt")
    pub aliases: &'static [&'static str],
    /// executable names without extension, in order of preference
    pub exes: &'static [&'static str],
    /// official install command, shown when the agent is missing
    pub install: &'static str,
    /// args that put the interactive TUI on a first prompt, ending where the prompt goes; None: cannot take one
    pub interactive_prompt: Option<&'static [&'static str]>,
    /// args before the prompt for a one-shot run that exits
    pub headless: &'static [&'static str],
    /// extra args that let a one-shot run edit files in the worktree without asking (safe level)
    pub unattended: &'static [&'static str],
    /// DANGEROUS: skips permission prompts. Off by default.
    pub dangerous: Option<&'static [&'static str]>,
    /// args that continue the latest conversation, put before everything else; None: cannot resume
    pub resume: Option<&'static [&'static str]>,
}

pub const TABLE: &[Agent] = &[
    Agent {
        id: "claude",
        name: "Claude Code",
        aliases: &[],
        exes: &["claude"],
        install: "npm install -g @anthropic-ai/claude-code",
        interactive_prompt: Some(&[]),
        headless: &["-p"],
        unattended: &[],
        dangerous: Some(&["--dangerously-skip-permissions"]),
        resume: Some(&["--continue"]),
    },
    Agent {
        id: "codex",
        name: "Codex",
        aliases: &["chatgpt"],
        exes: &["codex"],
        install: "npm install -g @openai/codex",
        interactive_prompt: Some(&[]),
        headless: &["exec"],
        unattended: &["--sandbox", "workspace-write"],
        dangerous: Some(&["--dangerously-bypass-approvals-and-sandbox"]),
        // a subcommand: `codex resume --last`
        resume: Some(&["resume", "--last"]),
    },
    Agent {
        id: "gemini",
        name: "Gemini CLI",
        aliases: &[],
        exes: &["gemini"],
        install: "npm install -g @google/gemini-cli",
        interactive_prompt: Some(&["-i"]),
        headless: &["-p"],
        // edits are approved, shell commands are not
        unattended: &["--approval-mode", "auto_edit"],
        dangerous: Some(&["--approval-mode", "yolo"]),
        resume: Some(&["--resume", "latest"]),
    },
    Agent {
        id: "opencode",
        name: "opencode",
        aliases: &[],
        exes: &["opencode"],
        install: "npm install -g opencode-ai",
        interactive_prompt: Some(&["--prompt"]),
        headless: &["run"],
        unattended: &[],
        dangerous: Some(&["--auto"]),
        resume: Some(&["--continue"]),
    },
    Agent {
        id: "aider",
        name: "Aider",
        aliases: &[],
        exes: &["aider"],
        install: "python -m pip install aider-install && aider-install",
        // no flag opens the chat on a prompt: --message answers once and exits
        interactive_prompt: None,
        headless: &["--message"],
        unattended: &[],
        dangerous: Some(&["--yes-always"]),
        resume: Some(&["--restore-chat-history"]),
    },
];

/// Installed state of one agent, as sent to the app by `agents.list`.
#[derive(Debug, Clone, serde::Serialize)]
pub struct AgentInfo {
    pub id: &'static str,
    pub name: &'static str,
    pub installed: bool,
    pub path: Option<String>,
    pub install: &'static str,
    pub can_resume: bool,
    pub interactive_prompt: bool,
}

pub fn get(id: &str) -> Option<&'static Agent> {
    TABLE.iter().find(|a| a.id == id || a.aliases.contains(&id))
}

/// The table entry for `id`, or a pt-PT error naming the valid ids.
pub fn require(id: &str) -> Result<&'static Agent> {
    match get(id) {
        Some(a) => Ok(a),
        None => bail!("agente desconhecido '{id}'; usa um destes: {}", ids().join(", ")),
    }
}

pub fn ids() -> Vec<&'static str> {
    TABLE.iter().map(|a| a.id).collect()
}

/// Extensions tried on Windows, where npm installs `.cmd` shims.
#[cfg(windows)]
const EXTS: &[&str] = &["exe", "cmd", "bat", "com"];
#[cfg(not(windows))]
const EXTS: &[&str] = &[""];

fn is_exec(p: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        p.metadata().map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0).unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        p.is_file()
    }
}

/// First match of any of `exes` in the directories of `path_var`.
pub fn find_in(exes: &[&str], path_var: &std::ffi::OsStr) -> Option<PathBuf> {
    for dir in std::env::split_paths(path_var) {
        for exe in exes {
            for ext in EXTS {
                let name = if ext.is_empty() { (*exe).to_string() } else { format!("{exe}.{ext}") };
                let p = dir.join(name);
                if is_exec(&p) {
                    return Some(p);
                }
            }
        }
    }
    None
}

impl Agent {
    pub fn locate(&self) -> Option<PathBuf> {
        std::env::var_os("PATH").and_then(|p| find_in(self.exes, &p))
    }

    /// Error text for a missing agent, with the install command.
    pub fn missing_message(&self) -> String {
        format!("{} não encontrado: instala com {}", self.name, self.install)
    }

    pub fn info(&self) -> AgentInfo {
        let path = self.locate();
        AgentInfo {
            id: self.id,
            name: self.name,
            installed: path.is_some(),
            path: path.map(|p| p.to_string_lossy().into_owned()),
            install: self.install,
            can_resume: self.resume.is_some(),
            interactive_prompt: self.interactive_prompt.is_some(),
        }
    }

    /// argv (program first) that opens the TUI, on `prompt` when the agent can take one.
    /// With `resume` it continues the latest conversation instead (the prompt is dropped).
    pub fn interactive_argv(&self, prompt: Option<&str>, resume: bool) -> Vec<String> {
        let mut v = vec![self.exes[0].to_string()];
        if resume {
            if let Some(r) = self.resume {
                v.extend(r.iter().map(|s| s.to_string()));
                return v;
            }
        }
        if let (Some(p), Some(flags)) = (prompt, self.interactive_prompt) {
            v.extend(flags.iter().map(|s| s.to_string()));
            v.push(p.to_string());
        }
        v
    }

    /// argv (program first) for a one-shot run on `prompt`. `unattended` adds the safe edit-without-asking
    /// flags, `dangerous` the permission-skipping ones; claude's own `--permission-mode` is the caller's.
    pub fn headless_argv(&self, prompt: &str, unattended: bool, dangerous: bool) -> Vec<String> {
        let mut v = vec![self.exes[0].to_string()];
        // `codex exec` and `opencode run` are subcommands: options come after them, the prompt last
        let sub = matches!(self.id, "codex" | "opencode");
        if sub {
            v.extend(self.headless.iter().map(|s| s.to_string()));
        }
        if unattended {
            v.extend(self.unattended.iter().map(|s| s.to_string()));
        }
        if dangerous {
            if let Some(d) = self.dangerous {
                v.extend(d.iter().map(|s| s.to_string()));
            }
        }
        if !sub {
            v.extend(self.headless.iter().map(|s| s.to_string()));
        }
        v.push(prompt.to_string());
        v
    }

    /// What a "start with this prompt" launcher runs: the TUI on the prompt, or a one-shot run for agents
    /// that cannot take a prompt in the TUI.
    pub fn start_argv(&self, prompt: &str) -> Vec<String> {
        if self.interactive_prompt.is_some() {
            self.interactive_argv(Some(prompt), false)
        } else {
            self.headless_argv(prompt, false, false)
        }
    }
}

pub fn list() -> Vec<AgentInfo> {
    TABLE.iter().map(Agent::info).collect()
}

/// `argv` with its program replaced by the full path found on PATH. A Windows `.cmd`/`.bat` shim cannot be
/// started directly, so it goes through `cmd /C`; only use that with arguments you wrote yourself.
pub fn resolve_argv(a: &Agent, mut argv: Vec<String>) -> Vec<String> {
    let Some(path) = a.locate() else { return argv };
    let p = path.to_string_lossy().into_owned();
    let shim = p.to_ascii_lowercase().ends_with(".cmd") || p.to_ascii_lowercase().ends_with(".bat");
    if shim && cfg!(windows) {
        argv.splice(0..1, ["cmd".to_string(), "/C".to_string(), p]);
    } else {
        argv[0] = p;
    }
    argv
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn lookup_and_aliases() {
        assert_eq!(get("claude").unwrap().name, "Claude Code");
        assert_eq!(get("chatgpt").unwrap().id, "codex");
        assert!(get("rm").is_none());
        assert!(get("").is_none());
        let e = require("rm -rf").unwrap_err().to_string();
        assert!(e.contains("desconhecido") && e.contains("gemini"), "{e}");
        assert_eq!(ids(), ["claude", "codex", "gemini", "opencode", "aider"]);
    }

    #[test]
    fn interactive_argv_per_agent() {
        let p = Some("faz isto");
        assert_eq!(get("claude").unwrap().interactive_argv(p, false), s(&["claude", "faz isto"]));
        assert_eq!(get("codex").unwrap().interactive_argv(p, false), s(&["codex", "faz isto"]));
        assert_eq!(get("gemini").unwrap().interactive_argv(p, false), s(&["gemini", "-i", "faz isto"]));
        assert_eq!(get("opencode").unwrap().interactive_argv(p, false), s(&["opencode", "--prompt", "faz isto"]));
        // aider has no way to open the chat on a prompt
        assert_eq!(get("aider").unwrap().interactive_argv(p, false), s(&["aider"]));
        assert_eq!(get("gemini").unwrap().interactive_argv(None, false), s(&["gemini"]));
    }

    #[test]
    fn resume_argv_per_agent() {
        assert_eq!(get("codex").unwrap().interactive_argv(Some("x"), true), s(&["codex", "resume", "--last"]));
        assert_eq!(get("gemini").unwrap().interactive_argv(None, true), s(&["gemini", "--resume", "latest"]));
        assert_eq!(get("claude").unwrap().interactive_argv(None, true), s(&["claude", "--continue"]));
        assert!(TABLE.iter().all(|a| a.interactive_argv(None, true).len() > 1));
    }

    #[test]
    fn headless_argv_per_agent() {
        let h = |id: &str, un, d| get(id).unwrap().headless_argv("P", un, d);
        assert_eq!(h("claude", true, false), s(&["claude", "-p", "P"]));
        assert_eq!(h("codex", true, false), s(&["codex", "exec", "--sandbox", "workspace-write", "P"]));
        assert_eq!(h("gemini", true, false), s(&["gemini", "--approval-mode", "auto_edit", "-p", "P"]));
        assert_eq!(h("opencode", true, false), s(&["opencode", "run", "P"]));
        assert_eq!(h("aider", true, false), s(&["aider", "--message", "P"]));
    }

    #[test]
    fn dangerous_flags_are_opt_in() {
        for a in TABLE {
            let safe = a.headless_argv("P", true, false).join(" ");
            assert!(
                !safe.contains("dangerously") && !safe.contains("yolo") && !safe.contains("--yes-always") && !safe.contains("--auto "),
                "{safe}"
            );
        }
        assert_eq!(get("claude").unwrap().headless_argv("P", false, true), s(&["claude", "--dangerously-skip-permissions", "-p", "P"]));
        assert!(get("gemini").unwrap().headless_argv("P", false, true).contains(&"yolo".to_string()));
    }

    #[test]
    fn prompt_stays_one_argument() {
        let evil = "a; rm -rf / && $(x) 'q' \"d\"";
        for a in TABLE {
            assert_eq!(a.headless_argv(evil, true, false).iter().filter(|x| x.as_str() == evil).count(), 1);
            assert_eq!(a.start_argv(evil).iter().filter(|x| x.as_str() == evil).count(), 1);
        }
    }

    #[test]
    fn start_argv_falls_back_to_one_shot() {
        assert_eq!(get("aider").unwrap().start_argv("P"), s(&["aider", "--message", "P"]));
        assert_eq!(get("claude").unwrap().start_argv("P"), s(&["claude", "P"]));
    }

    #[test]
    fn finds_executables_on_a_path() {
        let dir = std::env::temp_dir().join(format!("axelstra-agents-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let name = if cfg!(windows) { "gemini.cmd" } else { "gemini" };
        let f = dir.join(name);
        std::fs::write(&f, "x").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let path = std::env::join_paths([&dir]).unwrap();
        assert_eq!(find_in(&["gemini"], &path), Some(f));
        assert_eq!(find_in(&["opencode"], &path), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_message_has_the_install_command() {
        let m = get("gemini").unwrap().missing_message();
        assert_eq!(m, "Gemini CLI não encontrado: instala com npm install -g @google/gemini-cli");
    }
}
