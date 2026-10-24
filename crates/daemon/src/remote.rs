//! A daemon on another machine (WSL distro, ssh host), reached through `axelstra bridge` run there and spoken to over the
//! stdin/stdout of a child process. See docs/remote-daemon.md.
//!
//! Nothing here opens a network port: the client spawns `ssh`/`wsl.exe` with an argv (never a shell string) and the
//! bridge on the other side talks to the remote daemon's own pipe/socket as an ordinary user CLI.

use crate::client::Client;
use crate::proto::{encode, JSON};
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::io::{Read, Write};

/// First line the bridge writes to stdout. Whatever came before it (login scripts, notices) is discarded by the client.
pub const BANNER: &str = "AXELSTRA-BRIDGE/1";
/// How much output before the banner the client puts up with.
pub const BANNER_SCAN_MAX: usize = 64 << 10;
/// Program started on the remote when the spec does not say otherwise.
pub const DEFAULT_BIN: &str = "axelstra";

/// Before it answers `hello` a peer may send small frames only (as the daemon requires).
const FIRST_FRAME_MAX: usize = 64 << 10;

/// Where a remote daemon lives and how to reach it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteSpec {
    /// `wsl:<distro>`
    Wsl { distro: String, bin: String },
    /// `ssh:[user@]host[:port]`
    Ssh { user: Option<String>, host: String, port: Option<u16>, bin: String },
    /// Tests only: runs `program args...` with `env` added instead of ssh/wsl. `parse` never produces it, so no spec
    /// string, setting or flag can make the client start an arbitrary program.
    #[doc(hidden)]
    Exec { program: String, args: Vec<String>, env: Vec<(String, String)> },
}

fn is_ident(s: &str) -> bool {
    !s.is_empty() && s.len() <= 128 && !s.starts_with('-') && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

pub fn check_bin(bin: &str) -> Result<()> {
    let ok = !bin.is_empty()
        && bin.len() <= 200
        && !bin.starts_with('-')
        && bin.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '/' | '~'));
    if !ok {
        return Err(axelstra_core::coded!(
            "host.bad_bin",
            "invalid remote program {bin:?}: use letters, digits and . _ - / ~, not starting with '-'",
            bin = bin
        ));
    }
    Ok(())
}

fn check_port(p: &str) -> Result<u16> {
    if p.is_empty() || p.len() > 5 || !p.chars().all(|c| c.is_ascii_digit()) {
        return Err(axelstra_core::coded!("host.bad_port", "invalid port {p:?}", p = p));
    }
    match p.parse::<u16>() {
        Ok(n) if n >= 1 => Ok(n),
        _ => Err(axelstra_core::coded!("host.bad_port", "invalid port {p:?}", p = p)),
    }
}

impl RemoteSpec {
    /// Parses `wsl:<distro>` or `ssh:[user@]host[:port]` (host may be a bracketed IPv6 literal). Anything that could be
    /// read as an option, or that carries whitespace or control characters, is refused.
    pub fn parse(spec: &str) -> Result<RemoteSpec> {
        let spec = spec.trim();
        if spec.is_empty() {
            return Err(axelstra_core::coded!("host.spec_empty", "empty host spec (use wsl:<distro> or ssh:[user@]host[:port])"));
        }
        if spec.len() > 300 || spec.chars().any(|c| c.is_control() || c.is_whitespace() || !c.is_ascii()) {
            return Err(axelstra_core::coded!(
                "host.spec_chars",
                "invalid host spec: no spaces, control characters or non-ASCII characters allowed"
            ));
        }
        let bin = DEFAULT_BIN.to_string();
        if let Some(distro) = spec.strip_prefix("wsl:") {
            if !is_ident(distro) {
                return Err(axelstra_core::coded!("host.bad_distro", "invalid WSL distro name {distro:?}", distro = distro));
            }
            return Ok(RemoteSpec::Wsl { distro: distro.into(), bin });
        }
        let Some(rest) = spec.strip_prefix("ssh:") else {
            return Err(axelstra_core::coded!(
                "host.spec_unknown",
                "unknown host spec {spec:?} (use wsl:<distro> or ssh:[user@]host[:port])",
                spec = spec
            ));
        };
        let (user, hostport) = match rest.split_once('@') {
            Some((u, h)) => {
                if !is_ident(u) {
                    return Err(axelstra_core::coded!("host.bad_user", "invalid ssh user {u:?}", u = u));
                }
                (Some(u.to_string()), h)
            }
            None => (None, rest),
        };
        let (host, port) = if let Some(v6) = hostport.strip_prefix('[') {
            let Some((addr, tail)) = v6.split_once(']') else {
                return Err(axelstra_core::coded!("host.unterminated_bracket", "unterminated '[' in {hostport:?}", hostport = hostport));
            };
            if addr.is_empty() || !addr.contains(':') || !addr.chars().all(|c| c.is_ascii_hexdigit() || matches!(c, ':' | '.')) {
                return Err(axelstra_core::coded!("host.bad_ipv6", "invalid IPv6 address {addr:?}", addr = addr));
            }
            let port = match tail {
                "" => None,
                t => Some(check_port(
                    t.strip_prefix(':').ok_or_else(|| axelstra_core::coded!("host.after_bracket", "unexpected {t:?} after ']'", t = t))?,
                )?),
            };
            (addr.to_string(), port)
        } else {
            match hostport.split_once(':') {
                None => (hostport.to_string(), None),
                Some((h, p)) => (h.to_string(), Some(check_port(p)?)),
            }
        };
        let v6 = host.contains(':');
        if !v6 && !is_ident(&host) {
            return Err(axelstra_core::coded!("host.bad_host", "invalid ssh host {host:?}", host = host));
        }
        Ok(RemoteSpec::Ssh { user, host, port, bin })
    }

    /// The same spec running another program on the remote (`AXELSTRA_REMOTE_BIN`), for when `axelstra` is not on the
    /// PATH a non-interactive shell gets there.
    pub fn with_bin(mut self, new: &str) -> Result<RemoteSpec> {
        check_bin(new)?;
        match &mut self {
            RemoteSpec::Wsl { bin, .. } | RemoteSpec::Ssh { bin, .. } => *bin = new.to_string(),
            RemoteSpec::Exec { .. } => {}
        }
        Ok(self)
    }

    /// What to start: program, argv and extra environment. Always an argv, never a shell line.
    pub fn command(&self) -> (String, Vec<String>, Vec<(String, String)>) {
        match self {
            RemoteSpec::Wsl { distro, bin } => {
                ("wsl.exe".into(), ["-d", distro, "--exec", bin, "bridge"].map(String::from).to_vec(), Vec::new())
            }
            RemoteSpec::Ssh { user, host, port, bin } => {
                let mut a: Vec<String> = [
                    "-T",
                    "-o",
                    "BatchMode=yes",
                    "-o",
                    "ForwardAgent=no",
                    "-o",
                    "ForwardX11=no",
                    "-o",
                    "ClearAllForwardings=yes",
                    "-o",
                    "PermitLocalCommand=no",
                    "-o",
                    "ConnectTimeout=10",
                    "-o",
                    "ServerAliveInterval=15",
                    "-o",
                    "ServerAliveCountMax=3",
                ]
                .map(String::from)
                .to_vec();
                if let Some(u) = user {
                    a.extend(["-l".into(), u.clone()]);
                }
                if let Some(p) = port {
                    a.extend(["-p".into(), p.to_string()]);
                }
                // `--`: the host can never be taken for an option
                a.extend(["--".into(), host.clone(), bin.clone(), "bridge".into()]);
                ("ssh".into(), a, Vec::new())
            }
            RemoteSpec::Exec { program, args, env } => (program.clone(), args.clone(), env.clone()),
        }
    }
}

impl std::fmt::Display for RemoteSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RemoteSpec::Wsl { distro, .. } => write!(f, "wsl:{distro}"),
            RemoteSpec::Ssh { user, host, port, .. } => {
                f.write_str("ssh:")?;
                if let Some(u) = user {
                    write!(f, "{u}@")?;
                }
                if host.contains(':') {
                    write!(f, "[{host}]")?;
                } else {
                    f.write_str(host)?;
                }
                if let Some(p) = port {
                    write!(f, ":{p}")?;
                }
                Ok(())
            }
            RemoteSpec::Exec { program, .. } => write!(f, "exec:{program}"),
        }
    }
}

/// Reads from `r` up to and including the banner line, dropping whatever precedes it. Byte by byte on purpose: nothing
/// after the banner's newline may be consumed, the frames start there.
pub fn scan_banner(r: &mut impl Read) -> Result<()> {
    let mut line: Vec<u8> = Vec::new();
    let mut total = 0usize;
    let mut b = [0u8; 1];
    loop {
        if r.read(&mut b).context("reading the bridge's output")? == 0 {
            return Err(axelstra_core::coded!("host.bridge_closed", "the connection closed before the bridge announced itself"));
        }
        total += 1;
        if total > BANNER_SCAN_MAX {
            return Err(axelstra_core::coded!(
                "host.no_banner",
                "no bridge banner in the first {max} bytes: is `axelstra bridge` what runs there?",
                max = BANNER_SCAN_MAX
            ));
        }
        match b[0] {
            b'\n' => {
                if line.last() == Some(&b'\r') {
                    line.pop();
                }
                if line == BANNER.as_bytes() {
                    return Ok(());
                }
                if line.starts_with(b"AXELSTRA-BRIDGE/") {
                    return Err(axelstra_core::coded!(
                        "host.bridge_version",
                        "the remote bridge speaks {theirs:?}, this client {ours:?}: update the older side",
                        theirs = String::from_utf8_lossy(&line),
                        ours = BANNER
                    ));
                }
                line.clear();
            }
            c if line.len() < 256 => line.push(c),
            _ => {}
        }
    }
}

fn read_first_frame(r: &mut impl Read) -> Result<Value> {
    let mut len = [0u8; 4];
    r.read_exact(&mut len).context("the client went away before its hello")?;
    let len = u32::from_le_bytes(len) as usize;
    if !(1..=FIRST_FRAME_MAX).contains(&len) {
        bail!("bad first frame length {len}");
    }
    let mut body = vec![0u8; len];
    r.read_exact(&mut body)?;
    if body[0] != JSON {
        bail!("the first frame must be a hello request");
    }
    serde_json::from_slice(&body[1..]).context("the first frame is not JSON")
}

/// Copies until `from` ends or `to` fails; each chunk is flushed so interactive output is not held back.
fn pump(from: &mut dyn Read, to: &mut dyn Write) -> std::io::Result<()> {
    let mut buf = vec![0u8; 64 << 10];
    loop {
        let n = match from.read(&mut buf) {
            Ok(0) => return Ok(()),
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        to.write_all(&buf[..n])?;
        to.flush()?;
    }
}

/// The bridge: connects to the local daemon as the user's CLI (no agent credential), announces itself, answers the
/// client's `hello` with the daemon's, then copies bytes both ways. Returns `Ok` when the client closed its end, `Err` when
/// the daemon went away. The caller exits right after: the other direction's thread may still be blocked in a read.
pub fn run_bridge(pipe: &str, token: &str, mut input: impl Read + Send + 'static, mut output: impl Write + Send + 'static) -> Result<()> {
    let client = Client::connect_bridge(pipe, token)?;
    let hello = client.hello_result().clone();
    output.write_all(format!("{BANNER}\n").as_bytes())?;
    output.flush()?;
    let req = read_first_frame(&mut input)?;
    if req["method"] != "hello" {
        let body = json!({"id": req["id"], "error": "the first request must be hello"});
        output.write_all(&encode(JSON, body.to_string().as_bytes()))?;
        output.flush()?;
        bail!("the client's first request was {}, not hello", req["method"]);
    }
    let body = json!({"id": req["id"], "result": hello});
    output.write_all(&encode(JSON, body.to_string().as_bytes()))?;
    output.flush()?;

    let (mut from_daemon, mut to_daemon) = client.into_streams();
    let (tx, rx) = std::sync::mpsc::channel::<Result<()>>();
    let tx2 = tx.clone();
    std::thread::Builder::new().name("bridge-up".into()).spawn(move || {
        // stdin ended (or the daemon stopped listening): either way this bridge is done
        let r = pump(&mut input, &mut to_daemon);
        let _ = tx.send(r.map_err(|e| anyhow::anyhow!("writing to the daemon failed: {e}")));
    })?;
    std::thread::Builder::new().name("bridge-down".into()).spawn(move || {
        let r = pump(&mut from_daemon, &mut output);
        let _ = tx2.send(match r {
            Ok(()) => Err(anyhow::anyhow!("the daemon closed the connection (was it restarted?)")),
            Err(e) => Err(anyhow::anyhow!("the connection to the client or the daemon failed: {e}")),
        });
    })?;
    rx.recv().unwrap_or_else(|_| Ok(()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ssh_argv(spec: &str) -> Vec<String> {
        RemoteSpec::parse(spec).unwrap().command().1
    }

    #[test]
    fn parses_the_supported_forms() {
        assert_eq!(
            RemoteSpec::parse("wsl:Ubuntu-22.04").unwrap(),
            RemoteSpec::Wsl { distro: "Ubuntu-22.04".into(), bin: "axelstra".into() }
        );
        assert_eq!(
            RemoteSpec::parse("ssh:me@box.example.com:2222").unwrap(),
            RemoteSpec::Ssh { user: Some("me".into()), host: "box.example.com".into(), port: Some(2222), bin: "axelstra".into() }
        );
        assert_eq!(RemoteSpec::parse(" ssh:dev ").unwrap().to_string(), "ssh:dev");
        let v6 = RemoteSpec::parse("ssh:me@[::1]:22").unwrap();
        assert_eq!(v6.to_string(), "ssh:me@[::1]:22");
        assert_eq!(RemoteSpec::parse("ssh:[fe80::1]").unwrap().to_string(), "ssh:[fe80::1]");
    }

    #[test]
    fn refuses_anything_that_could_be_an_option_or_a_second_command() {
        for bad in [
            "",
            "   ",
            "ssh:",
            "wsl:",
            "ssh:-oProxyCommand=touch /tmp/x",
            "ssh:-oProxyCommand=touch",
            "ssh:-v",
            "ssh:me@-oProxyCommand=x",
            "ssh:-me@host",
            "ssh:host -o ProxyCommand=x",
            "ssh:host\nfoo",
            "ssh:host\tfoo",
            "ssh:host\0",
            "ssh:host;reboot",
            "ssh:host$(x)",
            "ssh:host`x`",
            "ssh:host|x",
            "ssh:host&x",
            "ssh:me@@host",
            "ssh:me@host:",
            "ssh:me@host:0",
            "ssh:me@host:65536",
            "ssh:me@host:+22",
            "ssh:me@host:2x",
            "ssh:a:b:c",
            "ssh:[::1",
            "ssh:[::1]x",
            "ssh:[zz::1]",
            "ssh:[]",
            "ssh:[host]",
            "ssh:@host",
            "ssh:host\u{00a0}x",
            "ssh:h\u{f6}st",
            "wsl:-d",
            "wsl:--exec",
            "wsl:Ubuntu --exec sh",
            "wsl:Ubu\nntu",
            "wsl:a/b",
            "wsl:a\\b",
            "telnet:host",
            "host",
            "ssh",
        ] {
            assert!(RemoteSpec::parse(bad).is_err(), "{bad:?} must be refused");
        }
    }

    #[test]
    fn ssh_argv_is_hardened_and_the_host_follows_double_dash() {
        let a = ssh_argv("ssh:me@box:2200");
        let pos = |x: &str| a.iter().position(|s| s == x).unwrap_or_else(|| panic!("{x} missing in {a:?}"));
        for opt in ["BatchMode=yes", "ForwardAgent=no", "ForwardX11=no", "ClearAllForwardings=yes", "PermitLocalCommand=no"] {
            assert_eq!(a[pos(opt) - 1], "-o");
        }
        assert!(a.contains(&"-T".to_string()));
        assert_eq!(a[pos("-l") + 1], "me");
        assert_eq!(a[pos("-p") + 1], "2200");
        let dd = pos("--");
        assert_eq!(&a[dd..], ["--", "box", "axelstra", "bridge"]);
        // no option of the user's may land after `--` except the host and the remote command
        assert!(a[..dd].iter().all(|s| s != "box"));
    }

    #[test]
    fn wsl_argv_runs_the_bridge_without_a_shell() {
        let (prog, args, env) = RemoteSpec::parse("wsl:Debian").unwrap().command();
        assert_eq!(prog, "wsl.exe");
        assert_eq!(args, ["-d", "Debian", "--exec", "axelstra", "bridge"]);
        assert!(env.is_empty());
    }

    #[test]
    fn the_remote_program_is_checked_too() {
        let s = RemoteSpec::parse("ssh:box").unwrap();
        let ok = s.clone().with_bin("~/.local/bin/axelstra").unwrap();
        assert_eq!(ok.command().1.last().map(String::as_str), Some("bridge"));
        assert!(ok.command().1.contains(&"~/.local/bin/axelstra".to_string()));
        for bad in ["", "-x", "a b", "a;b", "$(x)", "a\nb", "a'b", "a\"b", "a|b"] {
            assert!(s.clone().with_bin(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn banner_scan_skips_login_noise_and_stops_at_the_frames() {
        let mut data = b"Welcome to a box\r\nlast login: yesterday\nAXELSTRA-BRIDGE/1\n".to_vec();
        data.extend_from_slice(b"\x05\0\0\0\0abcd");
        let mut r = std::io::Cursor::new(data);
        scan_banner(&mut r).unwrap();
        let mut rest = Vec::new();
        r.read_to_end(&mut rest).unwrap();
        assert_eq!(rest, b"\x05\0\0\0\0abcd");
    }

    #[test]
    fn banner_scan_fails_clearly() {
        let e = scan_banner(&mut std::io::Cursor::new(b"bash: axelstra: command not found\n".to_vec())).unwrap_err();
        assert!(e.to_string().contains("closed before"), "{e:#}");
        let e = scan_banner(&mut std::io::Cursor::new(b"AXELSTRA-BRIDGE/2\n".to_vec())).unwrap_err();
        assert!(e.to_string().contains("update the older side"), "{e:#}");
        let e = scan_banner(&mut std::io::repeat(b'x')).unwrap_err();
        assert!(e.to_string().contains("no bridge banner"), "{e:#}");
        // a line that merely contains the banner is not it
        let e = scan_banner(&mut std::io::Cursor::new(b"xAXELSTRA-BRIDGE/1\n".to_vec())).unwrap_err();
        assert!(e.to_string().contains("closed before"), "{e:#}");
    }
}

#[cfg(test)]
mod code_tests {
    use super::*;
    use axelstra_core::msg::find;

    fn code(r: Result<RemoteSpec>) -> String {
        find(&r.unwrap_err()).unwrap().code.clone()
    }

    #[test]
    fn spec_errors_carry_codes() {
        assert_eq!(code(RemoteSpec::parse("")), "host.spec_empty");
        assert_eq!(code(RemoteSpec::parse("ftp:x")), "host.spec_unknown");
        assert_eq!(code(RemoteSpec::parse("wsl:a b")), "host.spec_chars");
        assert_eq!(code(RemoteSpec::parse("ssh:h:99999")), "host.bad_port");
        assert_eq!(code(RemoteSpec::parse("ssh:[::1")), "host.unterminated_bracket");
        let e = check_bin("-x").unwrap_err();
        assert_eq!(find(&e).unwrap().code, "host.bad_bin");
        assert_eq!(find(&e).unwrap().params["bin"], "-x");
    }
}
