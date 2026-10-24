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

## The app: host registry and routing (M7b)

The app (Tauri host, `app/src-tauri/src/hosts.rs`) keeps a **registry of named hosts**, `local` being implicit:
`{id, name, spec, slot, bin?}` in `settings.json` under `hosts` (max 16). The id is `[a-z][a-z0-9-]{0,23}`, the slot 1-127;
the daemon validates the list again (`settings.set`, `crates/daemon/src/hosts.rs`: spec through `RemoteSpec::parse`, never
the test-only `Exec`) and **refuses `hosts` to the agent role** (`agent_forbidden`). The page cannot change the registry
either: `settings.set` with `hosts` is refused by the host's `check_params`; only the Tauri commands `host_add` /
`host_remove` write it, each after a **native confirmation dialog**. `project_add_remote` is confirmed the same way.

Routing. Every daemon runs unchanged and keeps its own plain paths and ids. The host rewrites at the boundary, so the
page sees one flat world:

- a remote project's paths and project id carry the prefix `@<host>:` (`@lab:/home/me/proj`, project id `@lab:abc`), added to
  results and **stripped from params** under the keys `worktree(s) project cwd root main parent repo dir path(s) id` (a commit
  message that looks like a prefix is not touched; relative paths are not prefixed);
- a remote session id is `slot << 24 | id` (so ids stay unique and `session_write`/attach/kill know the host from the id);
- a call whose params name one host goes to that host's daemon (`control` or `bg` connection); local params are untouched;
  params that mix two hosts are refused. `project.list`, `session.list` and `session.pending` fan out to every connected host
  (8 s cap each, a host that is down only loses its own rows); `worktree.status`, `agent.worktree_usage` and `budget.status`
  split their path lists per host and join the arrays;
- terminals: `session_attach` opens its own bridge connection to the session's host and subscribes there; keystrokes use one
  writer thread **per host**, so a slow link never delays typing in a local terminal;
- events (`fs.changed`, `session.changed`, `projects.changed`, `session.exit`) come over one event connection per host and are
  forwarded, tagged, into the page's single event channel; `watch_set` points the stream at the host of the worktree.

Connection state per host (`idle | connecting | ok | lost | auth`, event `host-state`, command `hosts_list`): one keeper thread per
host connects at startup and again after a loss (waits grow 5 s ... 60 s; a refused login retries every 60 s). The event
connection is always open and its end is how a dead link is noticed (`ServerAliveInterval`). `daemon-restarted` is emitted
when a host's daemon is a new run (first contact included) so the page restores that host's saved sessions. `host_reconnect`
retries immediately. Errors keep ssh's own words (`Permission denied`, `Host key verification failed` are classed `auth`).

Connections per host: control, bg, input (opened on first use), the event connection and one per attached terminal; each is a
separate `ssh`/`wsl.exe` child (no ControlMaster: unavailable on Windows OpenSSH). Authentication is the user's ssh
configuration (keys/agent, `BatchMode=yes`), nothing is stored by Axelstra.

Security summary: the daemon token never leaves its machine (the bridge authenticates locally); `ssh`/`wsl.exe` are started with
an argv built from a strictly validated spec; the registry is writable only by the user (native confirmation, agent role
refused in the daemon, webview refused in the host); the agent role has no way to route: the daemon never connects to other
hosts and the routing exists only in the app's host process. Remote bridge connections are role `user` (see "Auth and roles").
A compromised page can still *use* a registered host (open its folders as projects, run commands in its terminals), as it can
on this machine; the registry itself and the `ssh` program line it cannot choose.

### Not supported for remote projects (honest list)

Refused with a clear message: Docker (`docker.*`), screen control (`computer.*`), "open in editor/Explorer" (`open.*`),
automations (`automation.*`: they live in the local daemon), hooks/backup/workspace export/settings (local). Not routed: the
agent usage/limits/search views (`agent.usage`, `agent.search`, `agent.status`, `codex.limits`) show **this machine's** logs;
`orch`/`kanban`/`triggers` of a remote project do run on its host (not exercised in the real tests). The page's path-based
native features (drag and drop of files, clipboard images) put **local** paths into a remote terminal. A remote daemon
is not started, installed or updated by the app. The registry is read at app start; editing `settings.json` by hand needs
a restart. No live PTY continuity across a dropped link (the sessions survive on the host; terminals re-attach).

## Implementation status

- Done: `RemoteSpec`, `Client::connect_remote`, `axelstra bridge`, `--host`; the host registry, routing, state, UI
  (see above); `host.info` / `host.browse` (PROTO 19); `Client::kill_child`.
- Verified for real on this Windows machine (see the test log in the M7b report): **WSL** (a Debian rootfs imported
  with `wsl --import`, axelstra built for Linux in a Debian container, daemon running inside the distro) and **SSH** (a Debian
  container running `sshd` with key auth and a throwaway key, daemon inside), both through the app host's routing code
  (`cargo test -p axelstra-app real_remote -- --ignored` with `AXELSTRA_TEST_HOST_SPEC` / `AXELSTRA_TEST_HOST_PROJECT`):
  connect + hello, `project.add`, `project.list` fan-out, `fs.list/read/write`, `git.changes/commit/log`, `worktree.create/remove`,
  terminal spawn + live output stream + input, `session.list`, `session.kill`, refusal of `docker.*`.
- Not verified: macOS and Windows-OpenSSH remotes, the GUI itself against a remote host (the page logic is covered by unit
  tests only), long-lived links (reconnect after a real network drop), ssh password/2FA (unsupported by design).
