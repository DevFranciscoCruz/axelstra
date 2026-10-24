# AGENTS.md

Notes for coding agents and new contributors.

## Layout

- `crates/core`, `crates/daemon`, `crates/cli`: engine (daemon, orchestrator, `axelstra` CLI).
- `crates/daemon/src/server.rs` holds the connection loop, the registry and `dispatch`, which routes a method by its
  prefix to `crates/daemon/src/server/{session,worktrees,files,vcs,containers,orchestration,messaging,misc}.rs`.
  A new method goes into the file of its domain; a new prefix also needs a line in the `dispatch` match.
- `app/src-tauri`: Tauri host (Rust). `app/src`: frontend (TypeScript, built with `npm`). Its state is split into
  `app/src/store/{ui,layout,sessions,notify,tabs,git,overlay}.ts` (re-exported by `app/src/store.ts`); `api.ts` is the only
  place that calls the daemon.
- `scripts/`: release and sidecar helpers and the check scripts below. `tools/`: smoke and E2E drivers, `fake-agent`.
  `docs/`: `TESTING.md`, `TOOLS.md` (scripts and tools), `E2E.md`, `RELEASING.md`, `DISTRIBUTION.md`, `s3-app-secret.md`.
- `.github/workflows/`: `ci.yml` and `release.yml`.

## Build and test

- Engine: `cargo test --no-fail-fast -p axelstra-core -p axelstra-daemon -p axelstra`
- Host: `cargo check -p axelstra-app` and `cargo test -p axelstra-app` (needs the desktop toolkit; on
  Linux install the webkit2gtk and gtk dev packages listed in `ci.yml`).
- Frontend: `cd app && npm ci && npx tsc --noEmit && npm test`
- Lint: `cargo fmt --all --check` and `cargo clippy --workspace --all-targets -- -D warnings`
- Release helpers: `node scripts/stamp-version.mjs --self-test` and `--check`.
- Everything at once: `node scripts/check.mjs` (fmt, clippy, tests, tsc, vitest; `--engine` skips the desktop toolkit).
- Check scripts: `scripts/check-tauri-config.mjs` (CSP and capabilities must stay as `SECURITY.md` says; no
  `style-src-attr 'unsafe-inline'`, so no static `style="..."`: use a class, or a CSS variable set from code),
  `scripts/check-format.mjs` (prettier on new or already formatted files; `prettier --write` them), `tools/check-deny-expiry.mjs`,
  `cd app && npm run lint` (eslint).
- E2E against the real app: `tools/e2e.mjs`, see `docs/E2E.md`. Smoke of the CLI and daemon: `tools/smoke.mjs`.
- **Tests never use the real data dir** (`%LOCALAPPDATA%/axelstra`) or the real pipe: use the helper in `crates/daemon/tests/common` and `AXELSTRA_DATA_DIR` (see `docs/TESTING.md`; `server::serve*` refuses the real dir under test, `scripts/check.mjs` verifies it).
- Heavy cargo/npm runs inside an Axelstra worktree go through `axelstra lock verify -- <cmd>`.

## Bumping PROTO

`PROTO` in `crates/daemon/src/proto.rs` is the app <-> daemon contract version. A daemon outlives
app updates, and the app only restarts a daemon older than `PROTO`. Bump it (and add the new method
to the doc comment above it) whenever the app starts calling a daemon method an old daemon lacks.
Do not bump it for changes an old daemon already tolerates. Current: 16 (notices.take, audit.list, backup.export,
backup.restore, orch.trust_info, env.tools, janitor.events).

## Worktree and agent lifecycle

- Every non-main worktree has a lead: its first fresh agent (`notes::pick_role`, called from `session.spawn`). It is
  named `main`, has `meta.lead = true`, takes no `Tarefa N` number (the counter counts tasks only and still restarts
  when the project has no running agent) and its task file is `main - <worktree folder>.md`. Later agents of that
  worktree get the lead as default parent (`AXELSTRA_PARENT`, `meta.parent`); the lead's parent is the project's main
  agent. A `worker: true` spawn, a named (restored) session and the main checkout never become leads. The app sends
  `lead` back on restore so a restart keeps the roles; the sidebar offers "Iniciar main do worktree" when none runs.
- A lead started by `axelstra dispatch new/send` is ephemeral: when it calls `axelstra done` its session closes (a
  reaper in mode `exit` with `lead: true`, so it does not count against the worker limit) and the report goes to main
  and to the task file. Main only reads the report.
- The janitor (`crates/daemon/src/janitor.rs`) sweeps every 10 minutes and a few seconds after each `done` or
  `git.merge` (`sweep_soon` in `server.rs`). A worktree goes when its `ax/*` branch tip is inside the default branch or
  inside another live `ax/*` branch (or its PR was merged), it has no changes, and no session works in it. A session
  that reported with `done` and is quiet for 10 s is not busy. Removals are queued as events (`janitor.events`) that
  the app shows as a toast and a notification-center entry.

Every method the frontend calls must also be in `ALLOWED_METHODS` in `app/src-tauri/src/main.rs`; a unit test reads
`app/src` (`call`, `callBg`, `callOr` with a string literal) and fails when the list and the sources drift apart.

## SECURITY_MIN

`SECURITY_MIN` in `proto.rs` is the oldest daemon the app accepts for security reasons, whatever its `PROTO`. `hello` and
`ping` report it (with `BUILD_ID`); the app restarts a daemon that reports less or nothing. Bump it when a release fixes
something in the daemon that must not keep running after an update (sessions are restored on restart). Agents get a
per-session token (`AXELSTRA_AGENT_TOKEN`) and the `role=agent` connection is refused the app-only methods listed in
`agent_forbidden` (server.rs): add new privileged methods there. `docs/s3-app-secret.md` explains what is still open.

## data_dir

`proto::data_dir()` is the per-user state directory: `%LOCALAPPDATA%\axelstra` on Windows,
`~/Library/Application Support/axelstra` on macOS, `$XDG_DATA_HOME/axelstra` (or
`~/.local/share/axelstra`) on Linux. It holds the daemon token, `locks/`, `flows/`, `crash.log` and
`lag.log`, and the state files `sessions.json`, `projects.json`, `settings.json` (also the UI choices kept for the webview
under `ui`), `automations.json`, each saved as a versioned envelope `{"v":N,"data":...}` with rotating `.1`..`.3` copies
(`store.rs`; a file that does not parse is moved to `.bad.<ts>`, never overwritten). `backup.export` copies the state
files into `backups/`, never the token. Cards use `serde_norway` (YAML). Tests must not write to the real one; point `LOCALAPPDATA`/`HOME`/`XDG_DATA_HOME` at a
temp dir.

## Versions and releases

Versions live in three places (Cargo workspace, `app/package.json`, `tauri.conf.json`) and must
agree; CI checks it. Push a `vX.Y.Z` tag on a commit that is already on `main`: `release.yml` runs
the whole CI, stamps the tag into the three files, builds, uploads to a draft release, merges
`latest.json` and only then publishes. The workflow's dry-run input does everything except publish.

## Git hygiene

Add paths explicitly (`git add <paths>`), never `git add -A`. `.kanban/` and `.axelstra/` are
intentionally ignored. There is no LICENSE file; do not add one without the owner's decision.
