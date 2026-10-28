# AGENTS.md

Notes for coding agents and new contributors.

## Layout

- `crates/core`, `crates/daemon`, `crates/cli`: engine (daemon, orchestrator, `axelstra` CLI).
- `crates/daemon/src/server.rs` holds the connection loop and `dispatch`, which routes a method by its
  prefix to `crates/daemon/src/server/{session,worktrees,files,vcs,containers,orchestration,messaging,misc}.rs`.
  The registry (`Registry`, `Pulse`, the save of `sessions.json`) is `server/registry.rs`, the background loops (reapers,
  scheduler, janitor, session monitor, stall watchdog) are `server/loops.rs`, the saved terminal history is
  `server/history.rs`. Everything typed into an agent's terminal is in `crates/daemon/src/prompts.rs`.
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
- Everything at once: `node scripts/check.mjs` (fmt, clippy, tests, tsc, vitest, `actionlint` on the workflows when it is on PATH; `--engine` skips the desktop toolkit).
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

## Awaiting something external

`axelstra await --reason "CI #3703" [--for-min N]` (`session.await`, `awaits` in `waiting.rs`/`Awaits`) marks the caller's own
session as awaiting: it only waits for CI, a deploy or another process. `session.list` carries
`awaiting: {reason, since, until}` (unix ms; `null` when not) and the app shows its own state, neither working, idle nor the
red waiting. `stall_facts` sets `awaiting`, so `is_stalled` is false meanwhile. It holds until the user types, output comes
more than `AWAIT_GRACE_MS` (30 s, the agent's closing words) after the call, `done`, the process exits, `axelstra await --clear`
or the limit (`--for-min`, default and cap `AWAIT_CAP_MS` 2 h); then the watchdog warns the parent once as usual. Event-driven
(`pulse` on set/clear, awaiting is part of the session monitor's state); no polling. An agent may only call it for itself
(`rpc.agent_own_session`). No PROTO bump: the app only reads an additive field. `NEVER_WAIT_RULE` (typed into every lead and
worker prompt) and the axelstration skill tell agents to use it instead of stopping silently.

## Bumping PROTO

`PROTO` in `crates/daemon/src/proto.rs` is the app <-> daemon contract version. A daemon outlives
app updates, and the app only restarts a daemon older than `PROTO`. Bump it (and add the new method
to the doc comment above it) whenever the app starts calling a daemon method an old daemon lacks.
Do not bump it for changes an old daemon already tolerates. Current: 29 (`gh.pr_refresh`; events `pr.changed {worktree, number, state, ci}`, `usage.changed`, `limits.changed`, `resources.changed` now emitted), before that 28 (events `clone.changed {id}` and `docker.changed {}` on the watch stream, the latter from a lazy `docker events` child; no new method), before that 27 (events `kanban.changed {project}`, `orch.changed {run, project}`, `automations.changed` on the watch stream; no new method), before that 26 (files.resolve_path, files.path_action), before that 25 (project.remove_preview and `mode` in project.remove), before that 24 (files.resolve_html), before that 23 (project.discover, project.add_discovered, project.clone, project.clone_status, project.clone_cancel,
project.init, gh.repos, wsl_distros/ssh_hosts in host.info, notices.take, audit.list, backup.export,
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
- **A session that ends on its own leaves the registry** (`Registry::retire`, called by `watch_exit`, and by the reaper
  of an ephemeral worker or lead after it reported to its parent). By event, no polling: it drops the row from
  `session.list`/`axelstra ls`, its spec from `sessions.json` (and its `sid/` and scrollback files), its bus state, then
  broadcasts `session.exited {id, code}` and `dispatch.changed` on the watch stream and bumps the session monitor and
  the janitor (`watch_exit` keeps doing the worktree activity note and the janitor kick). Only the caller that really
  removed the session announces, so a `session.kill`/`session.remove`/done-closer racing the exit says nothing (those
  already forget it themselves); a daemon shutdown (`closing`) retires nothing, the sessions stay saved for the restart.
  A `done` report is not lost: it is written to the parent's inbox and the task file when `done` is called, long
  before the exit; an ephemeral session is retired only after its reaper sent the report. `session.status` still
  answers for the last 256 retired sessions (`Registry::exited`, a leaf lock): `{running: false, code, gone: true,
  finished}`, so `axelstra wait`, flows (`orchcmd`) and the orchestrator runner get the exit code (a vanished id that is
  not remembered is `gone`/`session closed` as before). No PROTO bump: the app only reads an additive event; an old
  daemon keeps the old behaviour (the row stays with `running: false`). The app (`handleSessionExited` in
  `store/sessions.ts`, registered by `useBootstrap`) runs the usual done/stopped notifications on the last list it had,
  shows a non-zero code as an error toast (`L.notify.exitCode`) and keeps the tab of a failed terminal (not agents) as a
  local row until the user closes it (`dismissExited`): that row is never in `ls` or `sessions.json`.
- `integrate::integrate_with` also works when the default branch is checked out in no worktree (the main checkout sits on
  another branch): a fast-forward is `git update-ref refs/heads/<default> <tip> <old>` (compare-and-swap, no checkout
  touched); otherwise the merge runs in a throwaway detached worktree under the temp dir (always removed) and the ref is moved
  with the same CAS. A moved ref is `integrate.default_moved`; a conflict is `integrate.conflict` and changes nothing. When
  the default branch is checked out somewhere the old path (dirty check, merge there) is kept. `integrate.default_elsewhere` is gone.
- The lead is a dispatcher, not an implementer: its prompt (`FOOTER` in `crates/cli/src/dispatch.rs`) tells it to plan
  the request into subtasks, start workers in its own worktree (`axelstra agent . --report-to $AXELSTRA_SESSION`, they
  show as `Tarefa N` under `main`), not edit code or run heavy builds itself (only integrate commits and run the final
  `axelstra lock verify`), and `done` with commits, verified and not verified. Even a trivial request goes to one worker.
- The janitor (`crates/daemon/src/janitor.rs`) is woken by events (daemon start, a session ending, a few seconds after each
  `done` or `git.merge` via `sweep_soon` in `server.rs`, a finished flow or run, a retry backoff) and by timers: the
  earliest moment a hand-made worktree stops counting as touched (`MANUAL_QUIET_SECS` after the daemon started or after
  its last session left; `Registry::next_janitor_wake`, `JanitorTiming`), plus one remote deadline (30 min) that exists
  only while the last sweep found a managed worktree with own commits not yet integrated (`janitor::awaiting_remote`; a
  remote ref change bumps it). No slow tick and no fixed tight poll; kicks closer than a minute make one sweep. A worktree goes when its `ax/*` branch tip is inside the default branch or
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
- **Orchestrator runs (Kanban, `orch.start`).** A run's worktrees are Axelstra's own, so the janitor cleans them like an
  agent's (`janitor::managed_branch`: `ax/*`, `orch/<run>` integration branch, `orch-task/<run>/<card>` task branches;
  `integrable_branch` = `ax/*` and `orch/*`, the ones `integrate_with`, the overview's "por integrar" and `merged_reason`
  treat as work going into the default branch; a task branch only leaves when empty or inside a live `orch/*`/`ax/*`
  branch). `orch.start` marks `<run>-integ` and `<run>-<card>` `auto` before the run creates them (`orchend::mark_run`),
  and `Registry::orch_active` (part of `worktree_busy`) keeps the janitor off them while that run is `running`.
  For `orch/*` a merge in the reflog counts as an own commit (the integration branch only ever receives `merge --no-ff`).
  When the run's thread ends it calls `orchend::finish` (before the run state leaves `running`; again after the last
  `orch.retry` leaves nothing to review): a run with every card merged and unintegrated commits on `orch/<run>` is
  integrated into the default branch (`integrate_with`: fast-forward or clean merge, respecting the project's
  auto-integrate switch) and the worktree and branch go with it; on a conflict, a dirty checkout and the like it stays as
  "por integrar" with `integrate::attention` and the Integrar button (any later `done`, `git.merge` or sweep cleans up
  after a manual integration). An integration branch with nothing the default branch lacks (a failed run, a run that
  changed nothing) is removed at once; one with work after a partial run (failed, blocked or unreviewed cards) is kept
  with `orch.kept` and never integrated by itself; while a card waits for review nothing is touched (`orch.retry` needs
  both worktrees). Dirty worktrees never go. Old `orch/*` worktrees (no mark) are `manual` by `migrate_marks`: they
  leave only through real integration (`merged_reason`), never because they are empty. `worktree.sweep`, `stale_branches`
  and `project.remove` cover `orch/*` and `orch-task/*` under the same rules.

## Events, not timers

- **PR state and triggers.** The CI-fix trigger and PR state (`prwatch.rs`, `server/triggers.rs`) run on events, not a period: a
  ref moving in a project repo (`RefWatch`: `.git`, `FETCH_HEAD`, `packed-refs`, `refs/**`), the janitor pulse (done, `git.merge`,
  session exit, flow end), a change of the running-agent set, `gh.pr_refresh` (app window focus) and the app's own `git.checks`.
  Each wake re-reads (gh) only PRs whose last look is older than 10 s (5 min for merged/closed). The only timer is ONE computed
  deadline, the earliest due look: 60 s while an open PR's CI is pending, 15 min while open and settled, none otherwise. A PR that
  differs from the last look broadcasts `pr.changed` (the first look is silent); one no longer open also bumps the janitor.
  `gh.pr_refresh` is in `agent_forbidden`.
- **Usage, limits, resources.** `server/loops.rs` `watch_usage` (notify on `status/`, `~/.claude/projects`, `~/.codex/sessions`,
  `~/.gemini/tmp`, debounced) pushes `usage.changed`/`limits.changed` and wakes the budget loop (no 120 s timer).
  `resources.changed` rides with `session.changed` (no OS event exists for process RSS/CPU or child process spawn).
- **Threads.** Watcher threads sleep until an OS event, the debounce or `watcher::wake_stopped()`; any site that sets a watch stop
  flag must call it. `session.subscribe` has no heartbeat: streams end via `session::wake_streams` (closed connection or writer
  gone). The pump thread lingers until the next output because the pty `Receiver` cannot be woken.
- **Peer and messages.** `origin_of` validates the peer by pid plus process start time (`Peer` in `origin.rs`). `msg.send`
  broadcasts `session.changed` itself, so `ask`/`wait` never depend on the monitor tuple.
- **Sleeps kept on purpose.** The rest are bounded backoffs on external OS resources: AV/indexer rename locks (`fsops.rs`,
  `store.rs`), the git handle of a killed process (`git.rs`), cleanup in `sources.rs`, Windows `PeekRead` 1-10 ms (`client.rs`; the
  sync pipe has no readiness event), synthetic input settle (`computer.rs`), the runner trust menu (`runner.rs`), the docker events
  reattach backoff (`containers.rs`), the `daemon.shutdown` 300 ms flush (`misc.rs`; no ack exists), the pipe-appearance wait on
  daemon start (Tauri `main.rs`; no OS event) and the diagnostics `try_wait` 300 ms. Debounces (`sweep_soon`, `SAVE_DEBOUNCE`,
  `data_watch`, the budget 500 ms coalesce) are coalescing, not polling.

## Removing a project

`project.remove {id, mode?}` (app only: `project.remove` and `project.remove_preview` are in `agent_forbidden`). Mode `list`
(default, what old callers get) only drops the project from `projects.json`. Mode `all` is the full cleanup
(`server/removal.rs` for what needs the registry, `crates/daemon/src/removal.rs` for the rest), in this order: close every
session of the project (meta `project`, or cwd/worktree in the checkout or a worktree: agents, leads, workers, terminals, plus
the saved ones waiting for a restore) with `tear_down`, so the process trees die and nothing holds a folder; remove the
worktrees (`removal::removable_worktrees`: listed by git, not the main checkout, under `.axelstra-wt`, the only door to
deleting a folder) with `git::worktree_remove_strict` (rename-away, long paths via `remove_tree`, `worktree prune`) and delete
their `ax/*` branches (dirty or unmerged ones too: the app warned the user); drop the records (task files `notes/<id>/`,
`dispatch.json` entries, `triggers.json` entry, automations, the project's keys in settings `ui`, bus state per session,
flows, janitor events, integration marks); and only then the project from the list. The project's own checkout is never
touched. A folder still held by a process we do not own stays whole and is queued in `removals.json`; the janitor retries
the queue on its usual wakes (session exit, `sweep_soon`, daemon start, `worktree.sweep`) and with the bounded backoff
(`janitor::retry_delay`), never on a fixed timer, even when the automatic cleanup is off. `project.remove_preview {id}` is
read only: `{sessions, terminals, worktrees: [{path, branch, dirty, unmerged}]}`. The answer of `all` is
`{removed, sessions_closed, worktrees_removed, worktrees_pending, branches_deleted}`. Not touched on purpose: the
named locks in `locks/` (shared by all projects), the flow progress files (not tied to a project), `.kanban/` and
`.axelstra/` inside the repo, and the notifications and project preferences the webview keeps in its own storage.

Every method the frontend calls must also be in `ALLOWED_METHODS` in `app/src-tauri/src/main.rs`; a unit test reads
`app/src` (`call`, `callBg`, `callOr` with a string literal) and fails when the list and the sources drift apart.

## SECURITY_MIN

`SECURITY_MIN` in `proto.rs` is the oldest daemon the app accepts for security reasons, whatever its `PROTO`. `hello` and
`ping` report it (with `BUILD_ID`); the app restarts a daemon that reports less or nothing. Bump it when a release fixes
something in the daemon that must not keep running after an update (sessions are restored on restart). Agents get a
per-session token (`AXELSTRA_AGENT_TOKEN`) and the `role=agent` connection is refused the app-only methods listed in
`agent_forbidden` (server.rs): add new privileged methods there. `docs/s3-app-secret.md` explains what is still open.
Current: 3. Besides `agent_forbidden`, a `role=agent` connection is held to its own session tree: `INPUT` frames,
`session.subscribe`, `session.kill` and `session.remove` are allowed only on the agent's own session and on sessions it started
(`agent_drives` walks the bus parent chain, so a lead keeps driving its workers and flows keep cleaning theirs; a worker cannot
touch a sibling or its parent: it talks to them with `msg.*`). `worktree.remove` by an agent works only on its own worktree or
one it started, refuses a dirty or unintegrated one and deletes the branch with `-d`; the app keeps the forcing behaviour.
`agent.search` is limited to the agent's project. `session.pending` hands out a resume id from `sid/<key>.json` only when it is
UUID-shaped, because the app puts it into a shell line.

## data_dir

`proto::data_dir()` is the per-user state directory: `%LOCALAPPDATA%\axelstra` on Windows,
`~/Library/Application Support/axelstra` on macOS, `$XDG_DATA_HOME/axelstra` (or
`~/.local/share/axelstra`) on Linux. It holds the daemon token, `locks/`, `flows/`, `crash.log` and
`lag.log`, and the state files `sessions.json`, `projects.json`, `settings.json` (also the UI choices kept for the webview
under `ui`), `automations.json`, each saved as a versioned envelope `{"v":N,"data":...}` with rotating `.1`..`.3` copies
(`store.rs`; a file that does not parse is moved to `.bad.<ts>`, never overwritten). `backup.export` copies the state
files into `backups/`, never the token. Cards use `serde_norway` (YAML). Tests must not write to the real one; point `LOCALAPPDATA`/`HOME`/`XDG_DATA_HOME` at a
temp dir.

## Build metadata

`axelstra_core::buildinfo::{VERSION, COMMIT, BUILT}` come from `AXELSTRA_GIT_SHA` (a full sha is cut to 7 chars; else `git rev-parse --short=7 HEAD`,
else `unknown`) and `AXELSTRA_BUILD_DATE` (UTC `YYYY-MM-DD`, else today), set by `build-installer.ps1` and the workflows
(`github.sha`; shallow checkouts need it). `hello`/`ping`/`host.info` carry `version`, `commit`, `built` (additive, no PROTO bump;
missing = "unknown" in the app). Surfaces: `axelstra --version`/`-V`, `axelstra version --json`, footer, Settings > About
(daemon-vs-app mismatch warning, *Copy info* for bug reports). Details: `docs/TOOLS.md`, `docs/RELEASING.md`.

## Versions and releases

Versions live in three places (Cargo workspace, `app/package.json`, `tauri.conf.json`) and must
agree; CI checks it. Push a `vX.Y.Z` tag on a commit that is already on `main`: `release.yml` runs
the whole CI, stamps the tag into the three files, builds, uploads to a draft release, merges
`latest.json` and only then publishes. The workflow's dry-run input does everything except publish.

## Git hygiene

Add paths explicitly (`git add <paths>`), never `git add -A`. `.kanban/` and `.axelstra/` are
intentionally ignored. The project is MIT-licensed (`LICENSE`); do not change the license without the owner's decision.
