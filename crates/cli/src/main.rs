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
    /// List sessions held by the daemon
    Ls,
    /// Start a session in the daemon: `axelstra spawn -- cmd /k`
    Spawn {
        #[arg(last = true, required = true)]
        cmd: Vec<String>,
    },
    /// Kill a session
    Kill { id: u32 },
    /// Type into a session. Escapes: \e \r \n \xHH
    Send { id: u32, data: String },
    /// Print a session's output (history + live) for a few seconds
    Tail {
        id: u32,
        #[arg(long, default_value_t = 5)]
        secs: u64,
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

fn main() -> Result<()> {
    match Cli::parse().cmd {
        Cmd::Daemon => {
            use axelstra_daemon::{proto, server};
            let token = proto::load_or_create_token()?;
            // two workers is plenty: the heavy lifting is blocking PTY threads
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()?
                .block_on(server::serve(proto::pipe_name(), token))?;
        }
        Cmd::Ls | Cmd::Spawn { .. } | Cmd::Kill { .. } | Cmd::Tail { .. } | Cmd::Send { .. } => {
            use axelstra_daemon::{client::Client, proto};
            let mut c = Client::connect(&proto::pipe_name(), &proto::load_or_create_token()?)?;
            match Cli::parse().cmd {
                Cmd::Ls => println!("{}", c.call("session.list", serde_json::json!({}))?),
                Cmd::Tail { id, secs } => {
                    use axelstra_daemon::client::Frame;
                    c.call("session.subscribe", serde_json::json!({"id": id}))?;
                    std::thread::spawn(move || {
                        while let Ok(f) = c.read() {
                            if let Frame::Output { data, .. } = f {
                                print!("{}", String::from_utf8_lossy(&data));
                            }
                        }
                    });
                    std::thread::sleep(std::time::Duration::from_secs(secs));
                    std::process::exit(0);
                }
                Cmd::Send { id, data } => {
                    let mut out = Vec::new();
                    let b = data.as_bytes();
                    let mut i = 0;
                    while i < b.len() {
                        if b[i] == 0x5c && i + 1 < b.len() {
                            match b[i + 1] {
                                b'e' => out.push(0x1b),
                                b'r' => out.push(0x0d),
                                b'n' => out.push(0x0a),
                                b'x' if i + 3 < b.len() => {
                                    out.push(u8::from_str_radix(&data[i + 2..i + 4], 16)?);
                                    i += 2;
                                }
                                o => out.push(o),
                            }
                            i += 2;
                        } else {
                            out.push(b[i]);
                            i += 1;
                        }
                    }
                    c.send_input(id, &out)?;
                }
                Cmd::Kill { id } => {
                    c.call("session.kill", serde_json::json!({"id": id}))?;
                }
                Cmd::Spawn { cmd } => {
                    let r = c.call(
                        "session.spawn",
                        serde_json::json!({"program": cmd[0], "args": &cmd[1..], "cwd": std::env::current_dir()?}),
                    )?;
                    println!("{r}");
                }
                _ => unreachable!(),
            }
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
