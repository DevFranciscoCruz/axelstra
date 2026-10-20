use anyhow::Result;
use clap::{Parser, Subcommand};
use axelstra_core::{card, orch};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "axelstra", about = "Lightweight parallel-agent orchestrator")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run the background daemon that owns terminal sessions
    Daemon,
    /// List sessions: id, label, state (working/idle/done), worktree
    Ls {
        #[arg(long)]
        json: bool,
    },
    /// Start a session in the daemon: `axelstra spawn -- cmd /k`
    Spawn {
        #[arg(last = true, required = true)]
        cmd: Vec<String>,
    },
    /// Kill a session
    Kill { id: u32 },
    /// Register a git repo as a project
    AddProject { path: PathBuf },
    /// Set the command run in each new worktree of a project (empty clears it)
    SetSetup { id: String, setup: String },
    /// List projects and their worktrees
    Projects,
    /// Type into a session. Escapes: \e \r \n \xHH
    Send { id: u32, data: String },
    /// Install the /axelstration skill for Claude Code (~/.claude/skills)
    InstallSkill,
    /// Send a message to an agent session: pasted as one block, then Enter
    Say { id: u32, text: String },
    /// Print a session's output (history + live) for a few seconds
    Tail {
        id: u32,
        #[arg(long, default_value_t = 5)]
        secs: u64,
    },
    /// Manage worktrees of registered projects
    Worktree {
        #[command(subcommand)]
        cmd: WtCmd,
    },
    /// Start an agent (claude/codex) in a worktree as an app-visible session; prints the session id
    Agent {
        worktree: PathBuf,
        #[arg(long, default_value = "claude")]
        agent: String,
        #[arg(long)]
        prompt: Option<String>,
        /// Type this worker's output into session ID when done (no value: $AXELSTRA_SESSION)
        #[arg(long, num_args = 0..=1)]
        report_to: Option<Option<u32>>,
        /// When to report: exit or idle (default idle)
        #[arg(long)]
        when: Option<String>,
        #[arg(long)]
        idle_secs: Option<u64>,
        #[arg(long)]
        lines: Option<u32>,
    },
    /// Start a plain shell session in a worktree; prints the session id
    Terminal {
        worktree: PathBuf,
        #[arg(long)]
        title: Option<String>,
        /// Command to run in the shell (the shell stays open afterwards)
        #[arg(long)]
        cmd: Option<String>,
        /// Type this worker's output into session ID when done (no value: $AXELSTRA_SESSION)
        #[arg(long, num_args = 0..=1)]
        report_to: Option<Option<u32>>,
        /// When to report: exit or idle (default exit)
        #[arg(long)]
        when: Option<String>,
        #[arg(long)]
        idle_secs: Option<u64>,
        #[arg(long)]
        lines: Option<u32>,
    },
    /// One-shot worker: run a command in a worktree, report its output to a session when it exits
    Run {
        worktree: PathBuf,
        #[arg(long, num_args = 0..=1)]
        report_to: Option<Option<u32>>,
        #[arg(long)]
        lines: Option<u32>,
        #[arg(last = true, required = true)]
        command: Vec<String>,
    },
    /// Print the last lines of a session's screen/history, ANSI stripped
    Read {
        id: u32,
        #[arg(long, default_value_t = 200)]
        lines: usize,
    },
    /// Block until a session is quiet for N seconds or exited (exit 2 on timeout)
    Wait {
        id: u32,
        #[arg(long, default_value_t = 3)]
        idle_secs: u64,
        #[arg(long, default_value_t = 600)]
        timeout: u64,
    },
    /// Search past agent conversations (needs a daemon with agent.search)
    Search {
        text: String,
        #[arg(long, default_value_t = 20)]
        max: u32,
    },
    /// Show cards and the order they would run in
    Plan {
        #[arg(long, default_value = ".")]
        repo: PathBuf,
    },
    /// Run all `todo` cards in isolated worktrees and merge them
    Start {
        #[arg(long, default_value = ".")]
        repo: PathBuf,
        #[arg(long)]
        name: String,
        #[arg(long, default_value_t = 2)]
        max: usize,
        /// Where worktrees live (default: <repo>/../.axelstra-wt)
        #[arg(long)]
        wt_root: Option<PathBuf>,
        /// Run agents as interactive terminals in the daemon (start it with `axelstra daemon`)
        #[arg(long)]
        daemon: bool,
        /// Agent command, `{prompt_file}` is substituted. Everything after `--`.
        #[arg(last = true)]
        agent: Vec<String>,
    },
}

#[derive(Subcommand)]
enum WtCmd {
    /// Create a worktree; prints its path
    Create { project: String, name: String },
    /// Remove a worktree
    Rm { path: PathBuf },
}

mod ctl;

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Daemon => {
            use axelstra_daemon::{proto, server};
            #[cfg(target_os = "macos")]
            adopt_login_path();
            let token = proto::load_or_create_token()?;
            // two workers is plenty: the heavy lifting is blocking PTY threads
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()?
                .block_on(server::serve(proto::pipe_name(), token))?;
        }
        cmd @ (Cmd::Ls { .. }
        | Cmd::Spawn { .. }
        | Cmd::Kill { .. }
        | Cmd::Tail { .. }
        | Cmd::Send { .. }
        | Cmd::Say { .. }
        | Cmd::AddProject { .. }
        | Cmd::SetSetup { .. }
        | Cmd::Projects
        | Cmd::Worktree { .. }
        | Cmd::Agent { .. }
        | Cmd::Terminal { .. }
        | Cmd::Run { .. }
        | Cmd::Read { .. }
        | Cmd::Wait { .. }
        | Cmd::Search { .. }) => {
            use axelstra_daemon::{client::Client, proto};
            let c = Client::connect(&proto::pipe_name(), &proto::load_or_create_token()?)?;
            ctl::run(cmd, c)?;
        }
        Cmd::InstallSkill => {
            let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).ok_or_else(|| anyhow::anyhow!("no home directory"))?;
            let dir = PathBuf::from(home).join(".claude").join("skills").join("axelstration");
            std::fs::create_dir_all(&dir)?;
            std::fs::write(dir.join("SKILL.md"), include_str!("../../../skills/axelstration/SKILL.md"))?;
            println!("installed {} (use it as /axelstration in Claude Code)", dir.display());
        }
        Cmd::Plan { repo } => {
            let cards: Vec<_> = card::load_dir(&repo.join(".kanban"))?
                .into_iter()
                .filter(|c| c.meta.status == "todo")
                .collect();
            for id in card::topo_order(&cards)? {
                let c = cards.iter().find(|c| c.meta.id == id).unwrap();
                println!("{id}  agent={} deps={:?}", c.meta.agent, c.meta.depends_on);
            }
        }
        Cmd::Start { repo, name, max, wt_root, daemon, agent } => {
            let repo = axelstra_core::git::canon(&repo)?;
            let wt_root = wt_root.unwrap_or_else(|| repo.parent().unwrap().join(".axelstra-wt"));
            let r = orch::run(&orch::Options {
                repo,
                name,
                max_concurrency: max,
                wt_root,
                agent_override: if agent.is_empty() { None } else { Some(agent) },
                poll_ms: 500,
                runner: if daemon {
                    Some(std::sync::Arc::new(axelstra_daemon::runner::DaemonRunner::connect_default()?))
                } else {
                    None
                },
            })?;
            println!("integration: {} ({})", r.integ_branch, r.integ_wt.display());
            println!("merged: {:?}", r.merged);
            for (id, why) in &r.failed {
                println!("failed: {id}: {why}");
            }
            for (id, why) in &r.needs_review {
                println!("needs-review: {id}: {why}");
            }
            if !r.blocked.is_empty() {
                println!("blocked: {:?}", r.blocked);
            }
        }
    }
    Ok(())
}

/// Apps started from Finder get launchd's tiny PATH, so `claude`, `git` from Homebrew, `node`...
/// would not be found. Ask the user's login shell for the real one, once, at daemon start.
#[cfg(target_os = "macos")]
fn adopt_login_path() {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
    if let Ok(o) = std::process::Command::new(shell).args(["-ilc", "printf %s \"$PATH\""]).output() {
        let p = String::from_utf8_lossy(&o.stdout);
        let p = p.lines().last().unwrap_or("").trim();
        if o.status.success() && !p.is_empty() {
            std::env::set_var("PATH", p);
        }
    }
}
