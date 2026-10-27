# S3: telling the app from an agent

## The problem

An agent runs as the same OS user as the app. Anything the app can read, the agent can read: the `token` file in
`data_dir`, the environment it was given, even other processes' environments. A role derived from "who presents a
secret" cannot hold against a hostile agent: whatever secret the app uses, the agent can obtain it. An app-only
secret was considered and rejected:

1. The daemon outlives the app, so the next app instance must be able to re-authenticate without a secret passed at
   spawn. Any place it can read, the agent can read.
2. The CLI is also the user's tool (`axelstra project add` in a normal terminal), so it cannot be demoted.
3. Old CLIs and apps keep talking to whatever daemon is running.

Making files unreadable to the agent (ACLs) does not work either: same user, same SID. Only a different principal or
a sandbox (restricted token, AppContainer, separate uid) would, which is a different product.

## What is implemented (S3b): the role comes from the process, not from a secret

The daemon owns the agents' process trees. On `hello` it asks the OS who is on the other end of the connection
(`server/origin.rs`):

- **Windows:** `GetNamedPipeClientProcessId`, then `IsProcessInJob` against each session's Job Object. A job holds every
  descendant of the session's shell, also those whose parent died, and breaking away is not allowed.
- **Linux/macOS:** `SO_PEERCRED` / `LOCAL_PEERPID`, then process group and descendant check against the session
  leaders. On Linux the daemon is a child subreaper (`PR_SET_CHILD_SUBREAPER`), so a double-forked orphan is adopted by
  the daemon and found as "descends from the daemon". macOS has no subreaper: a process that fully detaches from its
  process group there can still pass as a user process (known limit).

A connection whose process belongs to a session is `role=agent` **whatever it sends**: the master token (read from
`data_dir` or from env) only authenticates, it never grants the user role. The session is the one that owns the
process, so `msg.send`/`msg.inbox`/`notes.append` identities cannot be spoofed with another session's credential
either. A client that is already gone when it is examined (it sent `hello` and exited) is treated as an agent
(`UNBOUND`), never as the user. Where the OS cannot tell the peer's pid, the old rule applies: `AXELSTRA_AGENT_TOKEN`
in `hello` gives the agent role.

Consequences:

- The app (not started inside a session) and the CLI in the user's own terminal keep the user role with the plain
  `token`. No second credential, so the daemon survives the app, app updates and restarts, and the CLI works the same
  way as before. Nothing new has to be stored or passed at spawn.
- Inside an Axelstra terminal the CLI is an agent: `project.add`, `computer.enable on`, `hooks.install`,
  `automation.save`, `orch.start`, `backup.*`, `notices.take`, `audit.list` are refused there (do them in the app). An
  app started from inside one of its own sessions (dev flow) is classified as that agent too.
- `computer.*` actions run in the daemon (`computer.rs`), behind the daemon's own switch: while it is off every action
  is refused for every client, and only the app can turn it on. Agent calls are audited.

Tests: `crates/daemon/tests/hardening.rs` (`a_process_started_by_a_session_cannot_get_the_user_role` spawns a session
whose process knows the master token and tries the app-only calls; `computer_actions_are_refused_by_the_daemon_...`).

## Migration and compatibility

- `SECURITY_MIN` is 2. An app that finds a daemon reporting less (an older release, still running) shuts it down and
  starts the current one; its sessions come back through session restore. That is the one-off cost of the update, and
  the reason an old daemon is never kept: it would trust anything holding the token file.
- Old app / old CLI against a new daemon: they send the same `hello`; the only difference is the role of processes
  inside sessions. `computer.*` calls of an old CLI (it injected input itself) still work through the old gate because
  the old CLI never calls the new `computer.*` actions; a new CLI against an old daemon fails with `unknown method`
  and the app replaces that daemon at its next start.
- `PROTO` is unchanged (15): no new call is needed by the app.

## What remains out of reach

A hostile agent can still use the OS directly (SendInput, reading files, `git`). The split protects the Axelstra
control plane (what the daemon, the registry, hooks and the machine's screen do on the user's behalf), not the
user's account. Real containment needs sandboxed sessions (restricted token / AppContainer / separate uid).

### Known bypasses of the process-based role

The role comes from which process holds the connection, so anything that starts a process outside the session's tree
reads the token file and is classified as the user:

- **Windows:** processes created through WMI (`Win32_Process.Create`, a child of `WmiPrvSE.exe`), COM activation
  (`Shell.Application`, `explorer`) or Task Scheduler (`schtasks`) do not inherit the session's Job Object.
- **Linux:** `systemd-run --user`, `at` and `cron` start processes that do not descend from the daemon.
- **macOS:** no process-tree check at all (see above).

An agent that knows one of these techniques gets user rights on the control plane, so every `agent_forbidden` guarantee
(`computer.enable`, `hooks.install`, `project.add`, `backup.restore`, `files.path_action`, ...) is a speed bump against
a confused or injected agent, not a barrier against a determined one. The CLI talks to the daemon directly, so the
app host never sees those calls and cannot ask the user to confirm them; a confirmation dialog for CLI clients would
need a daemon change and is not implemented. Containment of a hostile agent needs sandboxed sessions (see below).
