//! Dev servers a worktree's terminals are running: TCP ports in LISTEN state owned by the sessions'
//! processes or anything they started.

use anyhow::Result;
use axelstra_core::procs;
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

/// `/proc/net/tcp` or `/proc/net/tcp6` text: `(port, socket inode)` of every LISTEN row (state `0A`). The local
/// address is hex `ADDR:PORT` (8 hex digits for IPv4, 32 for IPv6); the inode is the 10th column. The header,
/// other states, inode 0 and malformed rows are skipped.
pub fn parse_proc_net_tcp(text: &str) -> Vec<(u16, u64)> {
    text.lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            if f.len() < 10 || f[3] != "0A" {
                return None;
            }
            let (addr, port) = f[1].rsplit_once(':')?;
            if !matches!(addr.len(), 8 | 32) || !addr.bytes().all(|b| b.is_ascii_hexdigit()) {
                return None;
            }
            let port = u16::from_str_radix(port, 16).ok()?;
            let inode: u64 = f[9].parse().ok()?;
            (inode != 0).then_some((port, inode))
        })
        .collect()
}

/// The inode of a `/proc/<pid>/fd/<n>` link target `socket:[12345]`; anything else is `None`.
pub fn socket_inode(link: &str) -> Option<u64> {
    link.strip_prefix("socket:[")?.strip_suffix(']')?.parse().ok()
}

/// Listening ports of `pids` read from `/proc`: no `lsof` needed (minimal images lack it). Only the fds of
/// the given pids are walked. `None` when `/proc/net/tcp*` cannot be read, so the caller falls back to lsof;
/// a pid that vanished or whose fds we may not read is skipped.
#[cfg(target_os = "linux")]
fn proc_listening(pids: &HashSet<u32>) -> Option<Vec<Port>> {
    use std::collections::HashMap;
    let mut by_inode: HashMap<u64, u16> = HashMap::new();
    let mut any = false;
    for f in ["/proc/net/tcp", "/proc/net/tcp6"] {
        if let Ok(t) = std::fs::read_to_string(f) {
            any = true;
            by_inode.extend(parse_proc_net_tcp(&t).into_iter().map(|(port, inode)| (inode, port)));
        }
    }
    if !any {
        return None;
    }
    let mut v = Vec::new();
    if by_inode.is_empty() {
        return Some(v);
    }
    for &pid in pids {
        let Ok(rd) = std::fs::read_dir(format!("/proc/{pid}/fd")) else { continue };
        for e in rd.flatten() {
            let Ok(target) = std::fs::read_link(e.path()) else { continue };
            if let Some(port) = target.to_str().and_then(socket_inode).and_then(|i| by_inode.get(&i)) {
                v.push(Port { port: *port, pid });
            }
        }
    }
    Some(v)
}

/// Listening TCP sockets straight from the IP helper table: no `netstat` process to spawn every few seconds.
#[cfg(windows)]
fn listening_table() -> Result<Vec<Port>> {
    use windows_sys::Win32::NetworkManagement::IpHelper::{GetExtendedTcpTable, TCP_TABLE_OWNER_PID_LISTENER};
    use windows_sys::Win32::Networking::WinSock::{AF_INET, AF_INET6};

    /// reads one address family: the table is `u32 count` followed by `count` rows of `row` bytes;
    /// the port is a network-order u16 in the low half of a u32 and the owning pid is the row's last u32
    fn family(af: u32, row: usize, port_at: usize) -> Result<Vec<Port>> {
        let mut size = 0u32;
        let mut buf: Vec<u32> = Vec::new();
        // the table can grow between the size query and the read: retry a few times
        for _ in 0..4 {
            let rc = unsafe {
                GetExtendedTcpTable(
                    if size == 0 { std::ptr::null_mut() } else { buf.as_mut_ptr().cast() },
                    &mut size,
                    0,
                    af,
                    TCP_TABLE_OWNER_PID_LISTENER,
                    0,
                )
            };
            match rc {
                0 => {
                    let bytes = buf.as_ptr().cast::<u8>();
                    let n = buf[0] as usize;
                    let mut v = Vec::with_capacity(n);
                    for i in 0..n {
                        let at = 4 + i * row;
                        if at + row > buf.len() * 4 {
                            break;
                        }
                        let u32_at = |o: usize| unsafe { std::ptr::read_unaligned(bytes.add(at + o).cast::<u32>()) };
                        v.push(Port { port: u16::from_be(u32_at(port_at) as u16), pid: u32_at(row - 4) });
                    }
                    return Ok(v);
                }
                122 | 0xFFFF_FFFF => buf = vec![0u32; (size as usize).div_ceil(4) + 16], // ERROR_INSUFFICIENT_BUFFER
                e => return Err(axelstra_core::coded!("port.table_failed", "GetExtendedTcpTable failed ({code})", code = e)),
            }
        }
        Err(axelstra_core::coded!("port.table_growing", "GetExtendedTcpTable kept growing"))
    }
    // MIB_TCPROW_OWNER_PID: state, local addr, local port, remote addr, remote port, pid
    let mut all = family(AF_INET as u32, 24, 8)?;
    // MIB_TCP6ROW_OWNER_PID: local addr[16], scope, local port, remote addr[16], scope, remote port, state, pid
    all.extend(family(AF_INET6 as u32, 56, 20).unwrap_or_default());
    Ok(all)
}

/// The port tools are polled every few seconds: a hung one must not pile up threads, and a huge table is cut.
fn run_tool(cmd: &mut Command) -> Result<axelstra_core::exec::Captured> {
    axelstra_core::exec::run_capped(cmd, std::time::Duration::from_secs(10), 8 << 20)
}

fn tool_failed(tool: &str, e: &anyhow::Error) -> anyhow::Error {
    axelstra_core::coded!("port.tool_failed", "could not run {tool}: {detail}", tool = tool, detail = format!("{e:#}"))
}

fn all_listening() -> Result<Vec<Port>> {
    if cfg!(windows) {
        #[cfg(windows)]
        if let Ok(v) = listening_table() {
            return Ok(v);
        }
        let o = run_tool(Command::new("netstat").args(["-ano", "-p", "TCP"])).map_err(|e| tool_failed("netstat", &e))?;
        Ok(parse_netstat(&o.stdout_lossy()))
    } else {
        // lsof exits 1 when nothing listens: not an error
        let o = run_tool(Command::new("lsof").args(["-nP", "-iTCP", "-sTCP:LISTEN", "-F", "pn"])).map_err(|e| tool_failed("lsof", &e))?;
        Ok(parse_lsof(&o.stdout_lossy()))
    }
}

/// Listening ports owned by `roots` or their descendants, one entry per port.
pub fn listening(roots: &[u32]) -> Result<Vec<Port>> {
    if roots.is_empty() {
        return Ok(Vec::new());
    }
    // the table is shared with the other pollers (memory, lag) instead of dumped again here
    let mine = procs::with_descendants_in(&procs::snapshot_cached(std::time::Duration::from_secs(2)), roots);
    #[cfg(target_os = "linux")]
    let found = match proc_listening(&mine) {
        Some(v) => v,
        None => all_listening()?.into_iter().filter(|p| mine.contains(&p.pid)).collect(),
    };
    #[cfg(not(target_os = "linux"))]
    let found: Vec<Port> = all_listening()?.into_iter().filter(|p| mine.contains(&p.pid)).collect();
    let mut v = found;
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
        assert_eq!(parse_lsof(s), vec![Port { port: 3000, pid: 10 }, Port { port: 3000, pid: 10 }, Port { port: 8080, pid: 11 }]);
    }

    #[test]
    fn proc_net_tcp_listen_rows() {
        let v4 = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode
   0: 0100007F:1F90 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 41234 1 0000000000000000 100 0 0 10 0
   1: 0100007F:C350 0100007F:1F90 01 00000000:00000000 00:00000000 00000000  1000        0 41299 1 0000000000000000 20 4 30 10 -1
   2: 00000000:0BB8 00000000:0000 0A 00000000:00000000 00:00000000 00000000     0        0 0 1 0000000000000000 100 0 0 10 0
   3: garbage
   4: 0100007F:ZZZZ 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 5 1
   5: 0100007:1F90 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 6 1
   6: 00000000:0BB8 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 notanum 1
";
        assert_eq!(parse_proc_net_tcp(v4), vec![(8080, 41234)]);
        let v6 = "  sl  local_address                         remote_address                        st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode
   0: 00000000000000000000000001000000:1435 00000000000000000000000000000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 77001 1 0000000000000000 100 0 0 10 0
   1: 00000000000000000000000000000000:0050 00000000000000000000000000000000:0000 06 00000000:00000000 00:00000000 00000000     0        0 0 3 0000000000000000
";
        assert_eq!(parse_proc_net_tcp(v6), vec![(5173, 77001)]);
    }

    #[test]
    fn socket_link_targets() {
        assert_eq!(socket_inode("socket:[41234]"), Some(41234));
        assert_eq!(socket_inode("pipe:[41234]"), None);
        assert_eq!(socket_inode("/dev/null"), None);
        assert_eq!(socket_inode("socket:[x]"), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn proc_finds_own_listener() {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let me = std::process::id();
        let got = proc_listening(&HashSet::from([me])).expect("/proc readable");
        assert!(got.contains(&Port { port, pid: me }), "{got:?}");
    }

    #[test]
    fn finds_a_port_this_process_listens_on() {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let got = listening(&[std::process::id()]).unwrap();
        assert!(got.iter().any(|p| p.port == port), "{got:?}");
    }
}
