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
- **Languages (i18n).** Every user-visible text lives in a typed dictionary: `app/src/strings*.ts` (pt-PT, the source: `S`, `P`, `B`, `R`, `W`, `WS`, `U` for components, `L` for logic modules) with a sibling `*.en.ts` typed `Widen<typeof pt...>` (a missing key is a compile error) and a complete pt-BR layer (`*.ptbr.ts`, Brazilian wording written out in full; a key missing there falls back to pt-PT but `i18n.test.ts` fails on it). `dict()` in `i18n.ts` makes reads follow the `locale()` signal, so the language changes live; the choice is stored under `axelstra.locale` (localStorage, mirrored to the daemon's settings `ui`). Never copy a dictionary text into a module-level `const` (use a function, `lazy`/`lazyPair`); `node scripts/check-i18n-static.mjs` (part of `scripts/check.mjs`) finds those. Plurals and numbers: `count`/`plural`/`fmtNumber`/`fmtFixed` from `i18n.ts`, never `toFixed`/`toLocaleString`. Dates and relative times go through `Intl` with the locale (`fmtDate`, `ago`, `stampLocal` in `time.ts`); files that leave the app (the timeline export) keep ISO (`stampIso`). `i18n.test.ts` and `i18nLint.test.ts` fail on missing keys, English copies of pt-PT and hardcoded literals in components.
- **Message codes (errors and notices from Rust).** A message the daemon, core or the Tauri host shows to the user is raised with `axelstra_core::coded!("domain.name", "English text with {param}", param = value)` (`crates/core/src/msg.rs`), never with a bare `bail!`/`anyhow!`. The English text is what the CLI and the logs print; on the wire the response also has `code` and `params` (additive: PROTO did not change, an old frontend shows `error`). The frontend turns them into text in its own language: `app/src/msg/<group>.ts` hold `pt`, `en` and `ptBR` for every code (same keys, same `{params}`), `AppError`/`errorText` read them, `coded(code, params, fallback)` does it for result fields and notices (`<field>_code`/`<field>_params`, `store::notice_coded`). A code is `domain.snake_case`; `FRIENDLY` in `friendly.ts` only catches what still arrives uncoded. `msg.test.ts` (vitest) and `msg::tests` (Rust) fail when a code raised in Rust lacks a pt-PT, English or pt-BR text, when the placeholders differ between languages or a key sits in two files. Add the code where the error is raised, the three texts in the group file, nothing else.
- **What stays English on purpose.** Prompts, rules and nudges typed into agents (`terse` rules, the dispatcher and worker prompts, the skill text, hook payloads, `axelstra agent --prompt`), because the agents are prompted in English and a changed prompt changes their behaviour; the CLI (`--help`, its output and errors: the `Display` text of a coded error is its English text); log files, `crash.log`, `lag.log`, `eprintln!`/`tracing` lines, the audit log's method names, diagnostics; branch/commit/PR titles and bodies that agents write; anything git, gh or docker print (shown as the `{detail}` of a translated sentence). Native dialogs of the Tauri host are translated by the host itself (`app/src-tauri/src/native.rs`, language read from the daemon's settings). Stored notifications keep the language they were born in; their relative times ("há 5 min") are formatted when shown.
## Waiting for an answer

`waiting` (red dot, bell, OS notification) is an event, never read off the screen text. Sources: `axelstra ask`/`escalate`
(`Bus::pending_of`: the question, until the reply or `done`) and the agent's own hook (`axelstra hook claude|gemini|codex EVENT`
reads the hook JSON on stdin and calls `session.hint {state, kind: permission|question|idle_input, text}`; `waiting::Hints`).
`idle_input` (Claude's `idle_prompt`) is only idle; a notification type we do not know, or a `waiting` hint without a kind, says
nothing. The hint clears on `Stop`/`UserPromptSubmit` (idle/working hints), on the user's input and on new output
(`hint_waiting`), and after 2 h. An agent connection may hint only its own session. `session.list` carries `wait: {kind, text}`;
the daemon tells the parent once per wait. A session whose agent has no hooks is only working or idle. Do not add screen
heuristics (`needs_input`/`asks_question` were removed on purpose); the stall watchdog (`stall.rs`) still covers silence.
Verified: Claude Code 2.1.310 Notification types (`permission_prompt`, `idle_prompt`, `elicitation_dialog`), Gemini 0.65.0
`ToolPermission`. Codex 0.154 has `~/.codex/hooks.json` events (`PermissionRequest`, `PreToolUse`, `Stop`, `UserPromptSubmit`);
`axelstra hook codex` understands them but nothing installs them (Codex asks the user to trust each hook).

## Bumping PROTO

`PROTO` in `crates/daemon/src/proto.rs` is the app <-> daemon contract version. A daemon outlives
app updates, and the app only restarts a daemon older than `PROTO`. Bump it (and add the new method
to the doc comment above it) whenever the app starts calling a daemon method an old daemon lacks.
Do not bump it for changes an old daemon already tolerates. Current: 22 (notices.take, audit.list, backup.export,
backup.restore, orch.trust_info, env.tools, janitor.events, workspace.export, workspace.import_preview,
workspace.import, host.info, host.browse, stalled in session.list, stall_minutes, and the watch-stream events janitor.events, triggers.events, budget.events, dispatch.changed).

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
- The lead is a dispatcher, not an implementer: its prompt (`FOOTER` in `crates/cli/src/dispatch.rs`) tells it to plan
  the request into subtasks, start workers in its own worktree (`axelstra agent . --report-to $AXELSTRA_SESSION`, they
  show as `Tarefa N` under `main`), not edit code or run heavy builds itself (only integrate commits and run the final
  `axelstra lock verify`), and `done` with commits, verified and not verified. Even a trivial request goes to one worker.
- The janitor (`crates/daemon/src/janitor.rs`) sweeps every 10 minutes and a few seconds after each `done` or
  `git.merge` (`sweep_soon` in `server.rs`). A worktree goes when its `ax/*` branch tip is inside the default branch or
  inside another live `ax/*` branch (or its PR was merged), it has no changes, and no session works in it. A session
  that reported with `done` and is quiet for 10 s is not busy. Removals are queued as events (`janitor.events`) that
  the app shows as a toast and a notification-center entry.
  Safe by default: automatic removal needs (1) an `auto` mark (`Project.auto`, set by `worktree.create` with `auto: true`,
  which `axelstra dispatch new` sends) or a branch really integrated with its own commits (`merged_reason`: no own
  commits, i.e. tip == default tip, never counts as merged), and (2) no session in the worktree for 10 min
  (`MANUAL_QUIET_SECS`; an `auto` worktree whose agent reported `done` is exempt). A worktree with no `auto` mark
  (old, hand-made, other projects, `worktree.from_pr`) counts as manual: it leaves only through real integration, and an
  empty one never leaves by itself (only the user's button). The "empty worktree" rule applies to `auto` only.
  `projects::migrate_marks` runs once at daemon start, before the janitor: worktrees that exist then with neither mark
  become `manual`; later ones are marked at creation. A removal that stays pending (folder held) is retried by event:
  a session exit (`watch_exit`) and `sweep_soon` kick the janitor. Holders we cannot observe get a bounded exponential
  backoff (`janitor::retry_delay`: 1, 2, 4... up to 30 min), never a fixed timer.

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
intentionally ignored. The project is MIT-licensed (`LICENSE`); do not change the license without the owner's decision.
