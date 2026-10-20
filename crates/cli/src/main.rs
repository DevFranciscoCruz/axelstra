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
        /// Agent command, `{prompt_file}` is substituted. Everything after `--`.
        #[arg(last = true)]
        agent: Vec<String>,
    },
}

fn main() -> Result<()> {
    match Cli::parse().cmd {
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
        Cmd::Start { repo, name, max, wt_root, agent } => {
            let repo = axelstra_core::git::canon(&repo)?;
            let wt_root = wt_root.unwrap_or_else(|| repo.parent().unwrap().join(".axelstra-wt"));
            let r = orch::run(&orch::Options {
                repo,
                name,
                max_concurrency: max,
                wt_root,
                agent_override: if agent.is_empty() { None } else { Some(agent) },
                poll_ms: 500,
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
