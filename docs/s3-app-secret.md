# S3 residual: an app-only secret

## Where S3 stands

Done on this branch / base:

- Every session gets its own capability token (`AXELSTRA_AGENT_TOKEN`). A `hello` that presents it makes the
  connection role=agent, bound to that session. `agent_forbidden` (server.rs) refuses it `computer.enable on`,
  `project.add`, `project.setup`, `hooks.install`, `automation.save`, `orch.start`, `notices.take`, `audit.list`.
- The sender of `msg.send` is the session the connection authenticated as; `msg.inbox` and `notes.append` are
  limited to the agent's own session (S9).
- Agent calls are recorded in the audit log (M13, `audit.list`).
- The token file is created private (`create_new`, mode 0600), compared in constant time, bad hellos are throttled (S17).

## What is still open

An agent runs as the same OS user as the app. Everything the app can read, the agent can read: the `token`
file in `data_dir`, the pipe, the app's memory. The role split is therefore advisory against a hostile agent: it
stops a well-behaved CLI and an agent that only follows its instructions, not one that opens `token` itself and
sends `hello` without `agent_token` (that connection is the "user" role).

## Why the secret handshake was not built here

The idea: the app starts the daemon and passes a secret (stdin or an inherited handle); the daemon accepts the
user role only from connections presenting it; the plain file token becomes role=agent-equivalent.

1. **The daemon outlives the app.** The app reconnects to a daemon it did not start (after an update, a restart of the
   window, a crash). It cannot pass a secret by stdin/handle to a process that is already running, so the secret
   would have to live somewhere the next app instance can read. Same user, same place the agent can read. Nothing gained.
2. **The CLI is also the user's tool.** `axelstra project add`, `axelstra daemon`, `axelstra lock`, the hook command
   and the helper scripts connect with the file token and no agent token when a human runs them in a terminal.
   Demoting the file token to a restricted role breaks all of them; they would need the app secret too, which
   leads back to 1.
3. **The old app and CLI keep working.** `PROTO`/`SECURITY_MIN` let the new app replace an old daemon, but an old CLI
   in `PATH` or a mixed install talks to whatever is running.

## What would actually separate the roles

The secret has to be unreadable by the agent at the OS level, so the agent must not run as the same principal with
the same ACLs:

- **Windows:** run agent sessions under a restricted token (`CreateRestrictedToken`, a deny-only user SID and
  write-restricted SIDs) or in an AppContainer, and put the `token` file and the pipe DACL on the full user SID only.
  The daemon then identifies the caller from the pipe (`GetNamedPipeClientProcessId` + token integrity) instead of
  from a bearer secret. The pipe already carries an explicit DACL (pipesec.rs) and the client already verifies the
  server (S6); the missing piece is the restricted-token spawn in `core::pty` (worker B's area).
- **Unix:** agents under a separate uid, or `SO_PEERCRED` on the socket plus a sandbox (bubblewrap / sandbox-exec)
  that hides `data_dir`.

Interim, cheap, and compatible: ACL the `token` file to the user SID with inheritance removed (S17 does this on Unix
with 0600; on Windows it inherits `%LOCALAPPDATA%`, which is already user-only), and keep the agent token role as the
default for anything an agent session starts (done).

## Decision needed

Whether sandboxing agent sessions (restricted token / AppContainer / separate uid) is in scope. Without it S3 stays
"mitigated, not enforced".
