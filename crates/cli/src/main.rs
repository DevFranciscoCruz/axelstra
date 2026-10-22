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
    /// Claude Code status line command used by Axelstra sessions (not meant to be run by hand)
    #[command(hide = true)]
    StatuslineTee,
    /// Print a bundled agent guide: `axelstra skills get axelstration`
    Skills {
        #[arg(value_parser = ["get"])]
        action: String,
        #[arg(value_parser = ["axelstration"])]
        name: String,
    },
    /// Install the /axelstration skill for Claude Code and Codex
    InstallSkill,
    /// Queue a message for another session; agents get a one-line notice, `axelstra inbox` reads it
    Msg {
        to: u32,
        text: String,
        /// conversation name, to follow related messages with `axelstra thread`
        #[arg(long)]
        thread: Option<String>,
        #[arg(long)]
        from: Option<u32>,
        /// do not type a notice into the recipient
        #[arg(long)]
        quiet: bool,
    },
    /// Read this session's messages (unread unless --all)
    Inbox {
        #[arg(long)]
        all: bool,
        /// leave the messages unread
        #[arg(long)]
        no_mark: bool,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        session: Option<u32>,
    },
    /// Ask another session a question and block until it answers (`axelstra reply`)
    Ask {
        to: u32,
        text: String,
        #[arg(long)]
        thread: Option<String>,
        #[arg(long, default_value_t = 600)]
        timeout: u64,
    },
    /// Answer message ID (the answer unblocks `axelstra ask`)
    Reply { id: u64, text: String },
    /// Worker: report the result to the parent session and finish (default parent: $AXELSTRA_PARENT)
    Done {
        text: String,
        /// ok, failed or blocked
        #[arg(long, default_value = "ok")]
        status: String,
        #[arg(long)]
        to: Option<u32>,
    },
    /// Worker: ask the parent for help; this session shows as waiting until answered
    Escalate {
        text: String,
        #[arg(long)]
        to: Option<u32>,
    },
    /// Show a whole conversation: every message sent with --thread NAME, in order
    Thread {
        name: String,
        #[arg(long)]
        json: bool,
    },
    /// Look at the screen and drive mouse and keyboard (the user must switch it on in the app first)
    Computer {
        #[command(subcommand)]
        cmd: CompCmd,
    },
    /// Show or set the maximum number of workers running at once
    Limit { n: Option<usize> },
    /// Run a task graph from a JSON file: {"tasks":[{"id","worktree","agent"|"cmd","prompt","after":[..]}]}
    Flow {
        file: PathBuf,
        #[arg(long, default_value_t = 3600)]
        timeout: u64,
        /// remove each task's worktree once the task succeeded, only when it is clean and its commits are pushed
        #[arg(long)]
        cleanup: bool,
        /// continue a plan that was interrupted (app closed): tasks that finished are skipped, the rest runs again
        #[arg(long)]
        resume: bool,
    },
    /// Send a message to an agent session: pasted as one block, then Enter
    Say { id: u32, text: String },
    /// Print a session's output (history + live) for a few seconds
    Tail {
        id: u32,
        #[arg(long, default_value_t = 5)]
        secs: u64,
    },
    /// Dispatcher (for the main session): route each request to the worktree of its topic, or open a new one
    Dispatch {
        #[command(subcommand)]
        cmd: DispCmd,
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
    /// Run a command when a slot of a named lock is free: agents work in parallel, the heavy command takes turns
    Lock {
        name: String,
        /// how many may hold the lock at once
        #[arg(long, default_value_t = 1)]
        slots: usize,
        /// seconds to wait for a slot
        #[arg(long, default_value_t = 3600)]
        timeout: u64,
        #[arg(last = true, required = true)]
        command: Vec<String>,
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
enum CompCmd {
    /// List visible windows: id, process, size and position, title
    Windows {
        #[arg(long)]
        json: bool,
    },
    /// Save a PNG of a window (--window ID) or of the whole screen; prints the path
    Screenshot {
        #[arg(long)]
        window: Option<u64>,
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Click at X Y (pixels from the window's top-left with --window, else from the screen's)
    Click {
        x: i32,
        y: i32,
        #[arg(long)]
        window: Option<u64>,
        #[arg(long, default_value = "left")]
        button: String,
        #[arg(long)]
        double: bool,
    },
    /// Type text into the focused window (or --window, which is focused first)
    Type {
        text: String,
        #[arg(long)]
        window: Option<u64>,
    },
    /// Press a key or combination such as enter, esc, ctrl+s, alt+f4
    Key {
        combo: String,
        #[arg(long)]
        window: Option<u64>,
    },
    /// Scroll at X Y: positive DY scrolls up, negative down
    Scroll {
        #[arg(allow_hyphen_values = true)]
        dy: i32,
        #[arg(long, default_value_t = 0, allow_hyphen_values = true)]
        dx: i32,
        #[arg(long, default_value_t = 0)]
        x: i32,
        #[arg(long, default_value_t = 0)]
        y: i32,
        #[arg(long)]
        window: Option<u64>,
    },
    /// Bring a window to the front
    Focus { window: u64 },
}

#[derive(clap::Subcommand)]
enum DispCmd {
    /// Open worktrees with title, areas touched, running agents and overlaps (decide where a request belongs)
    List {
        #[arg(long)]
        project: Option<String>,
        /// include merged and archived worktrees
        #[arg(long)]
        all: bool,
        #[arg(long)]
        json: bool,
    },
    /// Send a request to the agent of a worktree (path, branch or title); starts one if none is running
    Send {
        worktree: String,
        request: String,
        #[arg(long, default_value = "claude")]
        agent: String,
    },
    /// New topic: create a worktree and start an agent in it with the request
    New {
        project: String,
        name: String,
        request: String,
        #[arg(long)]
        title: Option<String>,
        #[arg(long, default_value = "claude")]
        agent: String,
    },
    /// Stop sending requests to a worktree (its work is done)
    Archive { worktree: String },
    /// Set the title shown for a worktree
    Title { worktree: String, title: String },
}

#[derive(Subcommand)]
enum WtCmd {

    /// Create a worktree; prints its path
    Create { project: String, name: String },
    /// Remove a worktree
    Rm { path: PathBuf },
    /// Remove the ax/* worktrees whose work is merged (branch in the default branch, or its PR merged on GitHub)
    Sweep {
        /// only list what would be removed
        #[arg(long)]
        dry_run: bool,
        /// keep doing it in the background every 10 minutes: on or off (no value: show)
        #[arg(long)]
        auto: Option<String>,
    },
}

mod computer;
mod lock;
mod statusline;
mod ctl;
mod dispatch;
mod orchcmd;

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Daemon => {
            use axelstra_daemon::{proto, server};
            #[cfg(target_os = "macos")]
            adopt_login_path();
            let _ = axelstra_daemon::skill::install();
            // a panic anywhere leaves a trace (the daemon's stderr goes nowhere)
            std::panic::set_hook(Box::new(|info| {
                let line = format!("{} panic: {info}\n", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0));
                let _ = std::fs::OpenOptions::new().create(true).append(true).open(proto::data_dir().join("crash.log")).and_then(|mut f| std::io::Write::write_all(&mut f, line.as_bytes()));
            }));
            let token = proto::load_or_create_token()?;
            // two workers is plenty: the heavy lifting is blocking PTY threads
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(4)
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
        | Cmd::Msg { .. }
        | Cmd::Inbox { .. }
        | Cmd::Ask { .. }
        | Cmd::Reply { .. }
        | Cmd::Done { .. }
        | Cmd::Escalate { .. }
        | Cmd::Limit { .. }
        | Cmd::Thread { .. }
        | Cmd::Computer { .. }
        | Cmd::Flow { .. }
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
        Cmd::Dispatch { cmd } => {
            use axelstra_daemon::{client::Client, proto};
            let c = Client::connect(&proto::pipe_name(), &proto::load_or_create_token()?)?;
            dispatch::run(cmd, c)?;
        }
        Cmd::Lock { name, slots, timeout, command } => std::process::exit(lock::run(&name, slots, timeout, &command)?),
        Cmd::StatuslineTee => statusline::tee()?,
        Cmd::Skills { .. } => print!("{}", axelstra_daemon::skill::guide()),
        Cmd::InstallSkill => {
            for f in axelstra_daemon::skill::install_forced()? {
                println!("installed {}", f.display());
            }
            println!("use it as /axelstration in Claude Code or Codex");
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
