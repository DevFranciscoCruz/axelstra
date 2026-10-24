# Remote daemon over SSH and WSL (M7)

One Axelstra app drives daemons on other machines: a WSL distro, or a host reached with `ssh`. Each remote host runs
its own daemon, its own projects, worktrees, agents and PTYs. The app only holds one more control connection.

## Transport: a stdio bridge, no network listener

```
app / CLI (this machine)                      remote machine
  Client ── child stdin/stdout ──  ssh -T host  ─────►  axelstra bridge ── pipe/socket ──► remote daemon
                          or      wsl.exe -d D --exec ─►
```

- `axelstra bridge` runs **on the remote**. It opens the remote daemon's local pipe/socket exactly as a normal user
  CLI does (`proto::pipe_name()`, token read from the remote's own `data_dir`), then copies frames between its
  stdin/stdout and the daemon. The frame protocol (`proto.rs`) is unchanged; **PROTO is not bumped**.
- The local side starts `ssh` or `wsl.exe` as a **child process** and speaks the frame protocol over the child's
  stdin/stdout (`Client::connect_remote`, `Conn::Child`).
- There is **no TCP port, no `ssh -L`, no forwarded listener**. A forwarded port would let any local process connect
  and be classified by the remote as the user (the remote sees the sshd/forwarder, not the caller). A child's pipes
  belong to the process that spawned it and nobody else.
- The daemon itself is never changed or exposed: it still listens only on its own pipe/socket (DACL / 0700 dir).

### Handshake

1. Bridge connects to the daemon, does `hello` itself with the remote token (no token ever crosses the wire), as
   `client:"cli"` and **without** an agent token.
2. Bridge writes one line `AXELSTRA-BRIDGE/1\n` to stdout. The client discards everything before it (login scripts
   that print to stdout, `wsl.exe` notices), at most 64 KiB, within the connect deadline. A different
   `AXELSTRA-BRIDGE/<n>` is refused with a clear message.
3. Client sends a normal `hello` frame (`{"client":"cli"}`, no token). The bridge answers it with the daemon's real
   `hello` result (proto, instance, build, security) under the client's request id, then relays bytes both ways.
   The bridge always connects as `cli`, whatever the client says: screen control (`computer.enable`) cannot be switched
   on through a bridge.
4. stdin EOF (client gone, child killed, ssh dropped) ends the bridge; daemon sessions keep running (as for any client).
   Daemon EOF ends the bridge with a message on stderr.

## Auth and roles (compatible with S3b)

The daemon decides the role from the **process on the other end of the connection** (`server/origin.rs`), and the
`hello` token only authenticates. Role of a bridge connection:

- peer pid = the `axelstra bridge` process. It was started by `sshd` (child of the sshd session) or by WSL's init
  (`wsl.exe -d D --exec ...`). It is not in any agent session's Job Object / process group and, on Linux, is not a
  descendant of the daemon (the subreaper check `orphan_of_ours`), so `origin_of(peer) == None`.
- the bridge presents no `agent_token` (it never reads `AXELSTRA_AGENT_TOKEN`, so a `SendEnv`/`WSLENV` leak is moot), so
  `agent = None`: **role = user** (with the `cli` marker). `project.add`, `backup.*`, `workspace.*` etc. work; they are
  the user's calls.
- Exact condition for the user role: `peer pid alive at hello` AND `not owned by any session` AND `not a descendant of
  the daemon (unix)` AND `no agent token`.
- If someone runs `axelstra bridge` from **inside an agent session** on the remote, the process is in the session tree,
  so the daemon classifies it `role=agent` whatever the bridge says. Fail-safe: the agent gets nothing more than it has.
  If the peer pid cannot be read or the process is gone, the daemon treats it as agent (existing behaviour).

### What this does not protect (honest)

- **Local agents can reach remote hosts.** An agent on this machine is the same OS user: it can run `ssh host` or
  `wsl.exe` and, through its own `axelstra --host ...`, would be the **user** on the remote, because the remote cannot
  see the local process tree. The agent could already run arbitrary commands on that host with the same `ssh`/`wsl`.
  The CLI therefore refuses `--host` when `AXELSTRA_AGENT_TOKEN` is set (agent session). That is defence in depth, not a
  boundary: an agent can unset the variable. Real separation needs a different principal or sandbox, as for S3.
- **Remote agents that leave their tree.** A remote agent that gets a process started by another supervisor (`ssh
  localhost`, `systemd-run --user`, cron) outside the daemon's tree is classified user, like the macOS "fully detached"
  limit in `s3-app-secret.md`. Linux orphans of the session are still caught by the subreaper.
- Trusting a host means trusting everything the ssh account can do there. Axelstra adds no sandbox.
- The remote **host key and user authentication are the user's ssh configuration**; nothing here weakens or replaces it.

## Spec and launch

`RemoteSpec` strings (`--host`, `AXELSTRA_HOST`, host registry):

| spec | launch (argv, never a shell string) |
| --- | --- |
| `wsl:<distro>` | `wsl.exe -d <distro> --exec <bin> bridge` |
| `ssh:[user@]host[:port]` | `ssh -T -o BatchMode=yes -o ForwardAgent=no -o ForwardX11=no -o ClearAllForwardings=yes -o PermitLocalCommand=no -o ConnectTimeout=10 -o ServerAliveInterval=15 -o ServerAliveCountMax=3 [-l user] [-p port] -- host <bin> bridge` |

`<bin>` defaults to `axelstra` (must be on the remote's non-interactive `PATH`; `AXELSTRA_REMOTE_BIN` overrides, charset
`[A-Za-z0-9_./~-]`, no leading `-`).

Validation (before anything is spawned): a spec is a single line; distro `[A-Za-z0-9._-]+`; host `[A-Za-z0-9._-]+` or a
bracketed IPv6 literal; user `[A-Za-z0-9._-]+`; port 1-65535; **no leading `-`** on any part, no whitespace, no control
characters, no empty part. `-oProxyCommand=...`, `host -o...`, `host\nfoo` and similar are rejected. `--` precedes the
host so it can never be an option. Host key checking, `ProxyJump`, identity files stay in the user's `~/.ssh/config`.
`BatchMode=yes` means no password or passphrase prompt ever reaches a hidden terminal: keys/agent only (the *local*
ssh-agent is used for authentication; agent *forwarding* is off).

## Path mapping

A project on a remote host keeps its **remote path** (e.g. `/home/me/proj`). The app never translates paths between
hosts. A project entry is `(host id, remote path)`; every daemon call for it goes to that host's daemon and carries the
remote path unchanged.

- The `\\wsl$\<distro>\...` (or `\\wsl.localhost\...`) view of a WSL filesystem is **display only**: handy for the user's
  Explorer, never used by Axelstra to read or write a project, because git and agents must run inside the distro.
- Remote host paths are never converted to local ones, and local paths are not valid on the remote.
- Opening a file or folder "externally" (editor, Explorer) is not available for remote projects.

## Failure modes

| situation | result |
| --- | --- |
| `ssh`/`wsl.exe` missing | spawn error naming the program |
| auth fails, host unreachable, host key unknown | ssh stderr (last 4 KiB) in the error; `BatchMode` never prompts |
| `axelstra` not on the remote PATH | shell stderr in the error ("command not found") |
| remote daemon not running | bridge exits 1: `daemon not reachable on this machine ... start it there`; the bridge never starts one |
| remote bridge older / no `bridge` subcommand | stderr of the remote plus "unrecognized subcommand" |
| no banner within the connect deadline (20 s) | child killed, error with stderr |
| call without answer in time | `CallTimeout`, child **killed**, connection marked broken (reconnect) |
| link drops (`ServerAliveInterval` ×3 ≈ 45 s) | ssh exits, client reads EOF, error with stderr |
| client dropped | child killed on drop; the bridge sees EOF; remote sessions keep running |

## Not supported (this pass)

- Windows OpenSSH server as the remote (the bridge is expected to work, untested), macOS remotes (unix socket path, untested).
- Starting the remote daemon, installing `axelstra` there, or updating it. The app does not restart a remote daemon;
  `PROTO`/`SECURITY_MIN` mismatch is reported, not fixed.
- Password / interactive auth, 2FA prompts, `ProxyCommand` needing a tty.
- File transfer, terminals via `axelstra` links to local files, port forwarding of remote dev servers, clipboard.
- Docker, `computer.*` (screen control), native dialogs and "open externally" for remote projects.
- Moving a project between hosts; the same remote path under two hosts is two projects.
- Reconnecting a live PTY stream transparently after the ssh link died: the sessions survive on the remote, the client
  reconnects and re-subscribes.
- A shared daemon between several users, or any exposure beyond the ssh account.

## Review notes on the recommended design

No flaw found in the stdio-bridge idea itself. Two additions: (1) the `AXELSTRA-BRIDGE/1` banner line, because login
scripts writing to stdout would otherwise corrupt the first frame; (2) the bridge performs the daemon `hello` itself and
fixes `client:"cli"`, so the token never leaves the remote and a remote caller cannot pick its own marker. The weakest
point is not the transport but the premise that "a process the daemon does not own is the user": see "What this does
not protect".
