//! `axelstra bridge` (runs on the remote machine) and `--host` (talks to a remote daemon through it).
//! See docs/remote-daemon.md.

use anyhow::{bail, Context, Result};
use axelstra_daemon::client::Client;
use axelstra_daemon::proto;
use axelstra_daemon::remote::RemoteSpec;
use std::time::Duration;

/// How long reaching a remote daemon (ssh login, bridge start, `hello`) may take.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);

/// The daemon for a command: the local one, or the one `host` names (`--host`, else `AXELSTRA_HOST`).
pub fn connect(host: Option<&str>) -> Result<Client> {
    let env = std::env::var("AXELSTRA_HOST").ok();
    let host = host.or(env.as_deref()).map(str::trim).filter(|h| !h.is_empty());
    let _ = CHOSEN.set(host.map(str::to_string));
    match host {
        None => Client::connect_cli(&proto::pipe_name(), &proto::load_or_create_token()?),
        Some(h) => Client::connect_remote(&remote_spec(h, in_agent_session())?, CONNECT_TIMEOUT),
    }
}

/// The host `connect` picked, so a command can open a second connection to the same daemon (its event stream).
static CHOSEN: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();

/// Another connection to the daemon the command already talks to.
pub fn connect_same() -> Result<Client> {
    match CHOSEN.get() {
        Some(h) => connect(h.as_deref()),
        None => connect(None),
    }
}

/// A session of an agent has its credential in the environment. Its commands must not reach other machines as the user
/// (the remote cannot tell them from the user's own). The variable can be unset by a hostile agent, so this is a guard
/// rail and not a boundary; docs/remote-daemon.md says so.
fn in_agent_session() -> bool {
    std::env::var("AXELSTRA_AGENT_TOKEN").is_ok_and(|t| !t.is_empty())
}

fn remote_spec(host: &str, agent: bool) -> Result<RemoteSpec> {
    if agent {
        bail!("--host is not available inside an agent session: remote hosts are the user's to drive");
    }
    let spec = RemoteSpec::parse(host)?;
    match std::env::var("AXELSTRA_REMOTE_BIN").ok().filter(|b| !b.is_empty()) {
        Some(bin) => spec.with_bin(&bin),
        None => Ok(spec),
    }
}

/// `axelstra bridge`: relays frames between stdin/stdout and the daemon of this machine. Stdout carries frames only, so
/// nothing else may be printed to it; errors go to stderr and the exit code.
pub fn bridge() -> Result<()> {
    // the token file is read, never created: a machine where no daemon ever ran has nothing to bridge to
    let path = proto::token_path();
    let token = std::fs::read_to_string(&path).map(|t| t.trim().to_string()).ok().filter(|t| !t.is_empty()).with_context(|| {
        format!("no daemon has run on this machine (no token at {}); start `axelstra daemon` or the Axelstra app here", path.display())
    })?;
    let pipe = proto::pipe_name();
    axelstra_daemon::remote::run_bridge(&pipe, &token, std::io::stdin(), std::io::stdout())
        .with_context(|| format!("axelstra bridge: the daemon at {pipe} is not usable (is it running on this machine?)"))?;
    // the relay threads may still be blocked on a read
    std::process::exit(0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_agent_session_cannot_pick_a_host() {
        let e = remote_spec("ssh:box", true).unwrap_err().to_string();
        assert!(e.contains("agent session"), "{e}");
        assert!(remote_spec("ssh:box", false).is_ok());
        assert!(remote_spec("ssh:-oProxyCommand=x", false).is_err());
    }
}
