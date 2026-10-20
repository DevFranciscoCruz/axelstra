//! Dev servers a worktree's terminals are running: TCP ports in LISTEN state owned by the sessions'
//! processes or anything they started.

use anyhow::{Context, Result};
use axelstra_core::{git, procs};
use serde::Serialize;
use std::collections::HashSet;
use std::process::Command;

#[derive(Serialize, Debug, PartialEq, Eq, PartialOrd, Ord, Clone)]
pub struct Port {
    pub port: u16,
    pub pid: u32,
}

/// `netstat -ano -p TCP` lines: `  TCP    0.0.0.0:3000   0.0.0.0:0   LISTENING   1234`
pub fn parse_netstat(out: &str) -> Vec<Port> {
    out.lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            if f.len() < 5 || !f[0].eq_ignore_ascii_case("tcp") || !f[3].eq_ignore_ascii_case("listening") {
                return None;
            }
            Some(Port { port: f[1].rsplit(':').next()?.parse().ok()?, pid: f[4].parse().ok()? })
        })
        .collect()
}

/// `lsof -F pn` output: `p1234` starts a process, `n*:3000` / `n127.0.0.1:3000` names a socket.
pub fn parse_lsof(out: &str) -> Vec<Port> {
    let mut pid = 0u32;
    let mut v = Vec::new();
    for l in out.lines() {
        if let Some(p) = l.strip_prefix('p') {
            pid = p.parse().unwrap_or(0);
        } else if let Some(n) = l.strip_prefix('n') {
            if let Some(port) = n.rsplit(':').next().and_then(|p| p.parse().ok()) {
                v.push(Port { port, pid });
            }
        }
    }
    v
}

fn all_listening() -> Result<Vec<Port>> {
    if cfg!(windows) {
        let o = git::quiet(&mut Command::new("netstat")).args(["-ano", "-p", "TCP"]).output().context("run netstat")?;
        Ok(parse_netstat(&String::from_utf8_lossy(&o.stdout)))
    } else {
        // lsof exits 1 when nothing listens: not an error
        let o = Command::new("lsof").args(["-nP", "-iTCP", "-sTCP:LISTEN", "-F", "pn"]).output().context("run lsof")?;
        Ok(parse_lsof(&String::from_utf8_lossy(&o.stdout)))
    }
}

/// Listening ports owned by `roots` or their descendants, one entry per port.
pub fn listening(roots: &[u32]) -> Result<Vec<Port>> {
    if roots.is_empty() {
        return Ok(Vec::new());
    }
    let mine = procs::with_descendants(roots);
    let mut v: Vec<Port> = all_listening()?.into_iter().filter(|p| mine.contains(&p.pid)).collect();
    v.sort();
    let mut seen = HashSet::new();
    v.retain(|p| seen.insert(p.port));
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn netstat_listening_only() {
        let s = "Active Connections\n\n  Proto  Local Address          Foreign Address        State           PID\n  TCP    0.0.0.0:3000           0.0.0.0:0              LISTENING       4242\n  TCP    [::]:5173              [::]:0                 LISTENING       4243\n  TCP    127.0.0.1:50000        127.0.0.1:3000         ESTABLISHED     77\n";
        assert_eq!(parse_netstat(s), vec![Port { port: 3000, pid: 4242 }, Port { port: 5173, pid: 4243 }]);
    }

    #[test]
    fn lsof_fields() {
        let s = "p10\nn*:3000\nn[::1]:3000\np11\nn127.0.0.1:8080\n";
        assert_eq!(
            parse_lsof(s),
            vec![Port { port: 3000, pid: 10 }, Port { port: 3000, pid: 10 }, Port { port: 8080, pid: 11 }]
        );
    }

    #[test]
    fn finds_a_port_this_process_listens_on() {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let got = listening(&[std::process::id()]).unwrap();
        assert!(got.iter().any(|p| p.port == port), "{got:?}");
    }
}
