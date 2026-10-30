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

## Hibernation

An agent session that really waits can be hibernated: the daemon ends its process tree and keeps the row. Pure parts
(setting, deadline, wake command, `Hib` bookkeeping) are in `crates/daemon/src/hibernate.rs`; the methods, the waker, the
policy thread and the list rows are in `server/hibernate.rs`.

- **State.** `Hib` (one leaf lock on `Registry`) holds the sleepers, the wake queue, the sessions being woken, an event counter
  per id and the coded error of a failed wake. A sleeper has no `Session` but keeps its id, label, `metas` row, saved spec
  (`specs`, so `sessions.json`) and bus state (inbox, parent, worker flag), so everything that names the id still finds it.
  `session.list` carries `hibernated: {since, saved_mb, waking, queued}` (`null` when awake; `running` stays `true`: it did not
  end) and the sleepers appear as rows. `session.status` answers `{running: true, hibernated: true}`. Events on the watch
  stream: `session.hibernated {id}`, `session.woke {id}`, `session.wake_failed {id, code, params, error}`.
- **Methods** (app only, both in `agent_forbidden`): `session.hibernate {id}` (idempotent, refuses with a coded `hibernate.*`
  error) and `session.wake {id, wait?}` (`state`: awake, waking, queued). `settings.get/set` carry `hibernate: {enabled (default
  true), idle_minutes (default 30, 1..1440)}`.
- **Never hibernated** (`check`): not an agent, the project main, no resume (claude needs a conversation with replies on disk,
  found through `sid/<key>.json`, `meta.resume` or the key; codex and gemini resume "the latest of the folder", so another
  session of the same agent in that folder refuses), input or news in the last `QUIET_MS` (20 s), output in the last 20 s,
  the hook said `working`, a wait is pending (permission, `ask`), or a process below the shell is a command, build, lock or hook
  (`hibernate::busy_process`, names from `procs::names_of`).
- **Race.** Every event about a session (`msg.send` to it, a wake, a gh comment for the lead, a hint-wait of its child) calls
  `Hib::touch(id)` BEFORE it looks the session up. `hibernate` reads the counter first, checks and saves with no lock held, then
  commits under the `sessions` lock: the session leaves `sessions` and enters the sleepers only if the counter did not move;
  otherwise it stays awake (`hibernate.event`). After the commit an event finds a sleeper and wakes it, its message already in
  the bus inbox. The old process' watcher (`watch_exit`) asks `hibernate::superseded` and does not retire the row.
- **Waking** is by event only: `msg.send` to it (done of a child, reply, ask, message), the reaper report of a child
  (`ReaperTask::finish` wakes the parent and waits for it to settle), `session.subscribe`, typing into it (`INPUT`; the keys are
  lost and `session.input_dropped` says so), `session.wake`. One waker thread serves a FIFO queue and sleeps on the pulse; a
  worker needs a free slot (`bus.limit()`), a lead or plain agent does not, and a sleeper is not in `live_workers`. A freed
  slot, a limit change or a new ask is a pulse. The wake spawns `wake_command` under the SAME id (`Registry::spawn_as`): the
  agent's shell with `claude --resume <id>` (flags `--settings`, `--append-system-prompt`, `--permission-mode`, `--model` copied
  verbatim from the original line), `codex resume --last` or `gemini --resume latest`, ending `|| exit 97` (`-NoExit` shells:
  `if (!$?) { exit 97 }`), so a failed resume is seen at once instead of falling back to a fresh start. The saved terminal is
  seeded, the worker flag and the reaper are armed again, then `settle` waits (a computed deadline: banner printed and 2.5 s
  quiet, 4 s at least, 90 s at most) and types the notices of the unread inbox. Only then does `hibernated` go back to `null`.
  If the process exits before that, or the spawn fails, the session is retired as exited with `wake_error`
  (`hibernate.wake_failed`), the parent is told and the watch stream gets `session.wake_failed`; it never stays hibernated.
- **Policy.** One thread (`spawn_policy`) computes ONE deadline per session (`hibernate::due_in`): it triggers when the session
  is awaiting (`axelstra await`), or is a lead with a live child, or reported `done` and was not retired, and has been quiet
  (no output, no input, no event) for `idle_minutes`. It sleeps on the pulse until the earliest deadline; state changes are
  pulses. A refused attempt backs off with `janitor::retry_delay`. No periodic tick. Tests shorten the timings with
  `AXELSTRA_HIBERNATE_QUIET_MS` and `AXELSTRA_HIBERNATE_IDLE_MS`.
- **Restart.** A sleeper's spec has `meta.hibernated`; `adopt` (at start) moves such records out of `pending` into new sleepers
  (new ids, lead/worker roles kept, parent and inbox lost like all bus state), so the app does not restore them.
- **Janitor.** `worktree_busy`/`worktree_touched` count a sleeper in the worktree, so its worktree is not cleaned.
- **Memory governor.** Hibernate is not pause: hibernate ends the process tree (RAM is freed, the agent resumes from its saved conversation), pause (`mem.pause`, `Suspender`) only freezes it and frees nothing. A paused session (or one running a frozen build) is refused with `hibernate.paused` (`Pauser::paused_of`); a commit that raced a pause continues the tree first (`resume_before_kill`). `mem.pause` on a sleeper answers `hibernate.sleeping`. A sleeper is not in `pause_candidates` (no `Session`), so the governor never picks it. At Critical, before it pauses anything, the pause round (`pause.rs::round`) tries to hibernate one session per round (`pause::sleepers`: agents by tier, workers first, biggest first; not terminals, builds or the project main) with the normal `session.hibernate` checks, so only really idle, resumable sessions sleep (RAM is freed); only when none can does it pause as before. It respects `hibernate.enabled` and `auto_pause`/`warn_only`; a level change ends the wait between rounds (event).
- **Spawn queue.** `session.spawn {queue: true, ticket?}` (additive, no PROTO bump: an old daemon ignores it and refuses as before) waits in a FIFO (`spawnq.rs`, `Registry::spawnq`, a leaf lock) instead of failing when the refusal is `session.worker_limit` or `governor.admission_denied`. Only the head of the line is checked, so nobody passes an earlier caller; the call sleeps on the pulse (a freed slot, a limit change, a memory level change, a woken sleeper) and, after one chunk (20 s), answers `{queued, ticket, position, reason}`; the caller comes back with the ticket and keeps its place. The place is given up when the call ends (`LinePlace` guard), after the session exists in `sessions`, so the next one never counts a slot that is already taken. A ticket whose caller stopped coming back is dropped when next looked at (`spawnq::STALE`, 90 s), no timer. `axelstra agent` (`ctl::spawn_agent`) sends `queue` and loops on `queued`: each call blocks in the daemon, so it is not a retry loop with a sleep. Other `session.spawn` callers (the app, the orchestrator) are unchanged. A woken sleeper uses its own FIFO (`Hib`), not this one.
- **App.** `hibernated` in `session.list` gives the dot state `hibernated` (`agentState.ts`; tip says since when, MB freed, waking, queued), a sidebar menu entry Hibernar/Acordar and a wake chip on the row (`sidebar/AgentList.tsx`), the Settings section (`Settings.tsx`: enabled, idle minutes), and `store/hibernate.ts` (toasts for `session.wake_failed` and `session.input_dropped`; the list refreshes on `session.changed`). Texts: `strings.hibernate{,.en,.ptbr}.ts` (`H`), codes `msg/hibernate.ts`.
- **Not done on purpose.** Triggers (CI fix) still see a worktree with only a sleeper as having no agent. The "focused tab" is
  approximated by recent input/subscribe events; the app does not tell the daemon which tab is focused.

## Bumping PROTO

`PROTO` in `crates/daemon/src/proto.rs` is the app <-> daemon contract version. A daemon outlives
app updates, and the app only restarts a daemon older than `PROTO`. Bump it (and add the new method
to the doc comment above it) whenever the app starts calling a daemon method an old daemon lacks.
Do not bump it for changes an old daemon already tolerates. Current: 32 (`session.hibernate {id}`, `session.wake {id, wait?}`, `hibernated` in session.list, `hibernate` in settings, events `session.hibernated`/`session.woke`/`session.wake_failed`; app only), before that 31 (memory governor: `mem.status`, `mem.config`, `mem.pause`, `mem.resume`, events `memory.pressure`, `session.paused`, `session.resumed`; app only for the config/pause ones), before that 30 (`orch.cancel {project}`: stops the run, the thread wakes by event, kills the workers' sessions, the unfinished cards go back to `todo`, `orchend::finish` never integrates a cancelled run (`orch.kept` when it left work); `integrate.undo {project, default_branch, prev_tip, new_tip, branch, branch_tip}`: takes back a `worktree.integrate` by compare-and-swap, only while the default branch is still at `new_tip`, not pushed and its checkout clean; both app only), before that 29 (`gh.pr_refresh`; events `pr.changed {worktree, number, state, ci}`, `usage.changed`, `limits.changed`, `resources.changed` now emitted), before that 28 (events `clone.changed {id}` and `docker.changed {}` on the watch stream, the latter from a lazy `docker events` child; no new method), before that 27 (events `kanban.changed {project}`, `orch.changed {run, project}`, `automations.changed` on the watch stream; no new method), before that 26 (files.resolve_path, files.path_action), before that 25 (project.remove_preview and `mode` in project.remove), before that 24 (files.resolve_html), before that 23 (project.discover, project.add_discovered, project.clone, project.clone_status, project.clone_cancel,
project.init, gh.repos, wsl_distros/ssh_hosts in host.info, notices.take, audit.list, backup.export,
backup.restore, orch.trust_info, env.tools, janitor.events, workspace.export, workspace.import_preview,
workspace.import, host.info, host.browse, stalled in session.list, stall_minutes, and the watch-stream events janitor.events, triggers.events, budget.events, dispatch.changed).

## Worktree and agent lifecycle

- **A lead is per request, not per worktree.** A worktree needs none, and a plain first agent is no longer promoted
  (`notes::pick_role`, called from `session.spawn`): only an explicit `meta.lead = true` makes a lead. A lead is named
  `main`, takes no `Tarefa N` number (the counter counts tasks only and still restarts when the project has no running
  agent) and its task file is `main - <worktree folder>.md`. Plain agents get the worktree's running lead as default parent
  (`AXELSTRA_PARENT`, `meta.parent`), else the project's main agent; the lead's parent is the project's main agent. A
  `worker: true` spawn, a named (restored) session and the main checkout never become leads. The app sends `lead` back on
  restore so a restart keeps the roles. The sidebar shows an existing lead first with a star and its tasks indented; a
  worktree of plain agents looks normal and nothing flags a missing lead ("Iniciar main do worktree" is an optional action
  next to existing agents, `canStartLead` in `leadRole.ts`). Old worktrees that have a lead are unchanged.
- **The main checkout never has tasks, it only dispatches them.** A project's own folder (or a folder below it, not under
  `.axelstra-wt`) holds only the project's main session and terminals. `session.spawn` from a `role=agent` connection or
  from the CLI (`axelstra agent`/`dispatch` send the additive `from: "cli"`; no PROTO bump) refuses there any agent that
  is not the project main (a task, a lead, a second `main` while one runs) and any worker that is no terminal, with the
  coded `session.main_no_tasks {project}` (the hint is `axelstra dispatch new <project> <name> "<request>"`; pure check
  `notes::refused_in_main_checkout`). `axelstra dispatch send` to the main checkout (its path, the project, its branch)
  refuses with the same code. The app is not refused by the daemon (it steers the user itself), so restored legacy
  `Tarefa N` sessions of a main checkout still restore.
- A lead started by `axelstra dispatch new/send --lead` (or by the heuristic) is ephemeral: when it calls `axelstra done` its
  session closes (a reaper in mode `exit` with `lead: true`, so it does not count against the worker limit) and the report
  goes to main and to the task file. Main only reads the report. A direct worker (the default) reports to the project main
  the same way.
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
  `axelstra lock verify`), and `done` with commits, verified and not verified. It never executes alone.
  `axelstra dispatch new/send` decide who starts: by default a direct worker (no lead; its parent is the project main; it
  integrates itself: merge main, `axelstra lock verify -- node scripts/check.mjs`, `done`); a lead when the request lists
  2+ independent subtasks (`wants_lead` in `crates/cli/src/dispatch.rs`); `--lead` / `--no-lead` override the heuristic.
  The stall watchdog ignores a lead that never got a request. Worktrees that already have a lead keep working as before.
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
  backoff (`janitor::retry_delay`: 1, 2, 4... up to 30 min), never a fixed timer. Integration with no ancestor counts (`cleanup::squash_merged`: `git cherry` has no `+` line, or merging the branch
  would change nothing), for `ax/*` and `orch/*`; a partly cherry-picked branch stays. A removal left pending because the
  folder is held is a janitor event `kind: "pending"`, once per episode (`janitor::pending_begin`/`pending_end`), with the
  holder (`janitor::holder_of`: Linux `/proc` cwd/exe, Windows an executable inside the folder) as `janitor.pending {branch,
  path, holder, pid}`, else the reason as `janitor.pending_busy {branch, path, detail}`. A worktree that stays (work not
  integrated) loses only its regenerable build output (`target/`, `node_modules/`, ignored caches) once inactive for
  `janitor.artifact_quiet_hours` or when the disk runs low: `artifacts.rs`, `disk.rs`, `server/disk.rs`, see `docs/DISK.md`.
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
- **Daemon calls (`client.rs`, Tauri host, CLI).** A pending call ends only by event: the reply, the connection closing
  (daemon gone, EOF, broken pipe, the `ssh`/`wsl.exe` child dying: coded `rpc.connection_closed {method}`; the client is then
  `broken` and the caller reconnects), never by elapsed time. There is no per-method default limit any more
  (`default_timeout` is gone). Liveness is state (process alive + pipe open); `ping`/`hello` only detect an outdated daemon
  (`PROTO`/`SECURITY_MIN`), they never declare it dead. The one clock left is a high safety ceiling for a lost handle:
  `client::ceiling()`, 600 s, `AXELSTRA_CALL_CEILING_SECS` to change it (min 1), also the bound of a blocked write and of the
  app waiting for a reconnect or an outdated daemon's exit (`rpc.call_ceiling {method, secs}`, connection dropped). Limits kept
  as policy, not watchdog: `Client::timeout` set by the caller (CLI `--timeout` flags, `list_host` of a remote host's sidebar
  list, 8 s), `policy_limit` for cosmetic reads (`project.logo`), `hook.rs` (an agent hook must never hold the agent up), the
  CLI's screen snapshot read (`recv_timeout` 1.5 s on a session that printed nothing), the app quitting (3 s on a daemon that
  ignores `daemon.shutdown`), `connect_remote`'s connect budget (ssh/WSL start; `ssh` also has `ConnectTimeout` and
  `ServerAlive*`, which close the stream when the link dies). Slow methods keep their lane (`is_slow`) and the method name in
  `rpc.call_timeout`/`rpc.call_ceiling`/`rpc.connection_closed`. Tests: `crates/daemon/tests/client_events.rs` (gated fake
  daemon: late reply succeeds, close mid-call fails at once, ceiling, explicit limit) and `remote.rs` (process killed).
- **Daemon calls, app side (`app/src/api.ts`).** `call`/`callBg` have no elapsed-time failure either: a pending call
  ends with the host's reply or error (`rpc.connection_closed`, `rpc.call_ceiling`, any coded error); the old 40 s / 130 s / 10 min
  guards and the `ping` after a late call are gone (`ping`/`hello` never declare the daemon dead). Slowness is not shown globally: there
  is no "system is slow" toast and no Cancel for a pending call; a view that loads slowly shows its own discreet loading state.
  Clocks kept as policy/net: `policyLimit` (cosmetic `project.logo`, 6 s, silent fallback in `logo.ts`) and
  `CEILING_MS` (600 s + 10 s margin after the host's `AXELSTRA_CALL_CEILING_SECS`, a lost invoke handle only; `rpc.call_timeout`).
  Also kept: `hydrateUi` 2 s (first paint must not wait for a daemon; the sync goes on), `NUDGE_LIMIT_MS` (a one-shot wait for an
  agent screen, not a call), UI debounces. The timeline panel's own 20 s `withTimeout` is gone.
  Tests: `apiRobust.test.ts` (fake transport: reply after 90 s succeeds, connection closed fails at once, ceiling, cosmetic).
- **Idle reads (`memo.rs`).** The slow polled reads (`worktree.overview/status`, `git.changes`, `dispatch.overlap_files`, `ports.list`, `gh.list`, `agent.worktree_usage`, `budget.status`, `project.list`) are cached per generation: events (`broadcast_event` names, RefWatch, fs watch, any method outside `may_change_answers`) bump it, identical in-flight calls join one run (single-flight), git work goes through a 3-permit pool (never call `known_worktree` while holding a permit), and at High/Critical the cached value is served stale. `max_age` floors exist only where no event exists. App side: `createLive` has `debounceMs`, `soft` (cheap signature) and skips events while hidden; `session.changed` must not drive loaders by itself. Idle check: `node tools/idle-load.mjs`.
- **Sleeps kept on purpose.** The rest are bounded backoffs on external OS resources: AV/indexer rename locks (`fsops.rs`,
  `store.rs`), the git handle of a killed process (`git.rs`), cleanup in `sources.rs`, Windows `PeekRead` 1-10 ms (`client.rs`; the
  sync pipe has no readiness event), synthetic input settle (`computer.rs`), the runner trust menu (`runner.rs`), the docker events
  reattach backoff (`containers.rs`), the `daemon.shutdown` 300 ms flush (`misc.rs`; no ack exists), the pipe-appearance wait on
  daemon start (Tauri `main.rs`; no OS event) and the diagnostics `try_wait` 300 ms. Debounces (`sweep_soon`, `SAVE_DEBOUNCE`,
  `data_watch`, the budget 500 ms coalesce) are coalescing, not polling.
- **Memory governor (`governor.rs`).** One thread waits on the OS memory signal (`MemSource::wait_change`); its only clock is ONE
  computed deadline (`next_deadline`: Normal = headroom to the High line divided by a worst-case 1 GB/s burn, clamped to 1 s..60 s
  with an OS event, 1 s..15 s without; not Normal = 1 s doubling to 10 s while nothing changes, reset on a level change). Admission
  waiters (`Governor::wait_change`, `mem.admit`) and the pause loop (`server/pause.rs`, `SETTLE` 2 s = the longest it waits to see
  whether the last pause helped, `BACKSTOP` 1 h = never blocks unbounded) sleep on the governor's generation counter, never on a
  tick. Details in "Memory governor" below.

## Memory governor

Code: `crates/daemon/src/governor.rs` (levels, config, `MemSource`, admission), `governor/{win,linux,sample,suspend}.rs` (one OS
source each, plus the `Suspender`), `server/governor.rs` (`mem.*` methods, the `memory.pressure` notifier), `server/pause.rs`
(victims, the table of what is frozen, the resume guarantees). CLI: `crates/cli/src/memcmd.rs` (`axelstra mem`, `axelstra resume`),
`wait_for_memory` in `crates/cli/src/lock.rs`. App: `app/src/store/memory.ts`, `memoryLogic.ts`, `MemoryPanel.tsx`, `strings.memory*.ts`.
User guide: `docs/MEMORY.md`. PROTO 31.

- **Levels.** `Normal < High < Critical`, from `MemInfo {total_mb, free_mb, commit_pct}` (`free_mb` = Windows `ullAvailPhys`, Linux
  `MemAvailable`). A level is entered when free memory is at or below the percentage OR the megabytes, and left only when it is above
  BOTH plus a margin (hysteresis, `next_level`, pure). Defaults: High at 20% or 3072 MB free, Critical at 10% or 1536 MB, margin 5% and
  512 MB (the margin is fixed, not configurable). `commit_pct` is reported, not used for the level. A failed read keeps the level.
  `enabled: false` pins Normal and announces nothing. Each change: `memory.pressure` + `resources.changed` (`server/governor.rs::notifier`).
- **Event sources.** Windows: `CreateMemoryResourceNotification` low/high objects plus a stop event in `WaitForMultipleObjects`, waiting on
  ONE object at a time (they stay signaled while the condition holds). Linux: PSI trigger `some 150000 1000000` on
  `/proc/pressure/memory` + `poll(2)` with a socketpair as the stop wake. macOS, and any OS whose event cannot be set up (`os_source`
  falls back to `Sampler`, logs to stderr): no event, the computed deadline is the only wake (`has_events() == false`, 15 s cap in
  Normal). The OS signals fire later than our lines (near exhaustion / on stall), so the deadline is what catches the lines; while
  the level is not Normal the deadline is short (1..10 s) because it is also the way back to Normal.
- **Config.** `settings.json` key `governor` (flat object, `Config`): `enabled`, `admission`, `auto_pause`, `warn_only` (bools),
  `high_free_pct`/`crit_free_pct` (1..90), `high_free_mb`/`crit_free_mb` (1..4194304); crit must be below high in both pairs.
  `mem.config {patch}` validates (`governor.bad_config`), saves, applies at once to the last reading and pokes the pause loop. A
  missing or broken `governor` key, or a bad single field, falls back to the default for that field. Unknown keys are errors.
- **Admission (`admission_denied`, pure; called in `session.spawn`).** Only at High and Critical, only when `gate_active()` (enabled,
  `admission`, not `warn_only`). Refused (`governor.admission_denied`): an AGENT started by an agent over an agent connection (not a
  worker); a worker once `live_workers >= effective_worker_limit` (High: half the `axelstra limit`, at least 1; Critical: 1). Never
  refused: terminals, anything the app or the user's CLI starts, workers under the limit. `axelstra lock` calls `mem.admit {wait_ms}`
  BEFORE taking a slot (a slot held while waiting would block builds already running) in chunks of 20 s; the daemon sleeps on the
  governor's generation and answers `{ok, level, free_mb}` the moment the level returns to Normal (or the gate goes off, the daemon
  closes); `lock` gives up with its usual `--timeout`. A daemon that cannot answer never holds a build back. Slots already held are untouched.
- **Pausing (`server/pause.rs`).** At Critical with `pause_active()` (enabled, `auto_pause`, not `warn_only`) one victim at a time is
  frozen, then the loop waits for a level change (`SETTLE` at most) before the next. Order (`victims`, pure): tier, then biggest RSS:
  1 builds/tests under `axelstra lock` (registered with `mem.hold {name, pid}`, only for a pid inside a session; an agent only for
  its own tree), 2 idle workers (quiet 20 s), 3 other agents (60 s), 4 leads (5 min), 5 the project main (5 min). Eligible only with
  RSS >= 32 MB and, for sessions, no input from the user in the last 2 min. Never: the daemon itself (and pids 0/1, in
  `OsSuspender`), terminals (a shell may host a server), a session the user typed into recently. A build ignores the input rule
  (nobody talks to it) and is preferred because a frozen agent's own network calls (API, git, MCP) can time out. A build whose
  process is gone or whose pid start time changed is not touched. `mem.pause {session}` is the user's manual freeze (reason
  `manual`); a level change never resumes it, only `mem.resume` or a kill does. Reason `memory` entries are resumed by event when
  the level leaves Critical, or the governor / `auto_pause` is switched off (`react`, `config` poke). `warn_only` at Critical
  broadcasts `memory.would_pause {targets}` and freezes nothing.
- **Suspender (`governor/suspend.rs`).** Trait so tests inject `FakeSuspender`. Unix `SIGSTOP`/`SIGCONT` per pid of the process tree (never
  the process group: the `axelstra lock` process and the agent's shell share it and must keep running); Windows `NtSuspendProcess`/
  `NtResumeProcess` (not `DebugActiveProcess`: it kills the target when the daemon dies). The tree comes from
  `procs::snapshot().descendants`. **Suspension frees no RAM**: it stops growth and lets the OS page the frozen processes out;
  the panel and docs must not promise otherwise.
- **Resume guarantees (nothing stays frozen).** Every pid `OsSuspender` stops is recorded first in a process-wide set;
  `resume_all_tracked()` continues all of them and is called by (1) `daemon.shutdown` after `reg.closing` is set, (2) the `ResumeGuard`
  `Drop` on `serve` (any exit), (3) the daemon's panic hook (`crates/cli/src/main.rs`; leaf lock only). Also `paused.json` in the data
  dir (pid + process start time, rewritten under a sequence number on each change, removed when empty): `Pauser::recover_stale` at
  start continues the pids a crashed daemon left frozen, only when the start time still matches (a reused pid is left alone).
  `session.kill`/`session.remove`, the done-closer reaper, `kill_sessions_where` (orphan reaper) and `tear_down` (project removal)
  call `resume_before_kill` (continues the session and the builds it runs) BEFORE killing, so a kill never waits on a frozen tree.
- **RPC.** `mem.status` (level, free/total, commit_pct, thresholds with margins, config, per-session RSS, `paused` list), `mem.config {patch}`,
  `mem.pause {session}`, `mem.resume {session | all}`, `mem.admit {wait_ms<=60000}`, `mem.hold {name, pid}`, `mem.unhold {id}`.
  Events on the watch stream: `memory.pressure {level, from, free_mb, total_mb, top: [{session, rss_mb}]}`, `memory.would_pause`,
  `session.paused {id, build, reason}`, `session.resumed {id}`, plus `resources.changed`. `session.list` rows carry
  `paused: {reason, since, build} | null`, `mem_level`, `mem_free_mb`. CLI: `axelstra mem [--json]`, `axelstra resume <session> | --all`.
- **`agent_forbidden`.** `mem.config`, `mem.pause`, `mem.resume`: an agent must not switch the governor off, move its lines, or freeze
  or unfreeze sessions (including a sibling or its own lead). `mem.status`, `mem.admit`, `mem.hold`, `mem.unhold` stay open because
  `axelstra lock` runs inside agents; `mem.hold` refuses a pid outside the agent's own tree (`not_own_tree`).
- **Cause of the pressure (`governor/cause.rs`).** `mem.status` carries an additive `breakdown` (no new method, no PROTO bump): `kernel`
  `{paged_mb, nonpaged_mb, total_mb, baseline_mb, high}` (Windows `GetPerformanceInfo`; Linux `SReclaimable`/`SUnreclaim`, alerting on the
  unreclaimable part only; macOS `null`), `cache_mb`, `compressed_mb` (Windows: the working set of `Memory Compression`, which is then not listed
  as external; Linux `Zswapped`), `sessions` per tier (`main|lead|agent|worker|terminal`: count and RSS), `builds` (processes under a `mem.hold` pid,
  counted apart from their session), `external` (top 8 by RSS, same-name processes grouped, everything outside the sessions), `others_mb`
  (memory in use, total minus free, minus the above; the cache is not subtracted because available memory already counts it as free; saturating),
  `handles {total, top: [{name,pid,handles}]}` (only when the kernel is high, the machine has over 200k handles or one process over 50k; Windows only),
  `system {cpu_user_pct, cpu_kernel_pct, commit_mb, commit_limit_mb, modified_mb, pages_per_sec}` and `top_cpu`/`top_io` `[{name,pid,value}]`
  (CPU % of the machine, I/O bytes/s; measured over a 300 ms window only while an alert is open, else `null`). Every source is a `CauseSource` (injected
  in tests, `Governor::cause.set_source`); the OS one uses direct APIs only (`GetSystemTimes`, `GetPerformanceInfo`, `NtQuerySystemInformation`,
  `GetProcessMemoryInfo/HandleCount/Times/IoCounters`, `/proc`), never PDH or `Get-Counter` (they fail on some installs). Processes the OS refuses to open
  (protected ones, e.g. Defender) show RSS 0; nothing ever acts on them.
- **Pool tags and our own share.** `breakdown.pool_tags` (Windows only, `null` elsewhere or when the OS refuses; additive, no PROTO bump):
  `NtQuerySystemInformation(22)` (`win::pool_tags`, no admin, buffer regrown on `STATUS_INFO_LENGTH_MISMATCH`) parsed by the pure
  `cause::parse_pool_tags` (u32 count, 4 pad, 40-byte entries: tag at 0, PagedUsed at 16, NonPagedUsed at 32). `top` = 8 biggest by bytes
  `{tag, paged_mb, nonpaged_mb, owner}`; `owner` is an id (`filter` FMfn, `tokens` Toke, `files` File, `watchers` Ntfc, `ntfs` Ntff, `mm` MmSt,
  `io` IoNm, `etw` EtwB) or `null`: unknown tags get no invented owner. `sem_etiqueta_mb` = kernel counters minus the sum of all tags (large
  allocations and special pools have no tag), `partial: true` always. Read only when the breakdown is built and when `governor.kernel_pool_high`
  is raised (no timer). The notice then names the biggest tag with one code per owner (`governor.kernel_pool_{filter,tokens,files,watchers,ntfs,mm,io,etw,tag}`);
  no tags (Linux, refused): the old generic code. Linux keeps `/proc/meminfo` Slab/SReclaimable/SUnreclaim only (slabinfo needs root).
  `breakdown.own` = `{spawns, uptime_secs, spawns_per_min (null under 60 s), watches}`: children started through `core::proc::hidden`
  (`spawn_stats`) and directories watched by `watcher.rs` (`core::proc::WatchTally`, dropped with the watcher thread; the few fixed
  watchers of kanban, PR refs, usage and orchestrator runs are not counted). Shown in `axelstra mem`, the memory panel and `debug.load`.
- **Kernel alerts.** No new timer: `Governor::set_tick` runs `server/governor.rs::cause_tick` on each wake of the governor thread (OS event or its
  computed deadline), which reads only the kernel pools and the machine counters (`cause::Monitor::evaluate`). Pool alert (`KernelWatch`): the first
  reading is the baseline; the line is the lower of `kernel_pool_mb` and twice the baseline (floor 1 GB); one `governor.kernel_pool_high {gb, handles}`
  notice (`store::notice_coded`, so `notices.take`) per episode, which ends when the pool falls below 85% of the line. Load alert (`LoadWatch`): the
  rates come from the delta between two evaluations; kernel CPU over 15% or over 5000 pages/s for 3 evaluations in a row raises one
  `governor.kernel_load_high {kernel_pct, pages}` notice, re-armed below 70% of both lines. The process table is read only when a notice is raised or
  `mem.status` is asked. `mem.config` accepts `kernel_pool_mb` (256..=1048576, default 3072, `governor.bad_config` otherwise).

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

## Indexer exclusion

Worktree folders (`.axelstra-wt`) are slow under file indexers. `crates/daemon/src/scanx.rs` keeps them out, with no admin, no
prompt, no UI and no daemon method: Windows sets `FILE_ATTRIBUTE_NOT_CONTENT_INDEXED` on `.axelstra-wt` and each worktree folder
(new content below a flagged folder inherits it); macOS puts an empty `.metadata_never_index` in `.axelstra-wt`; Linux does nothing.
Set by event: `projects::create_at` (`worktree.create`, `worktree.from_pr`), `orch.start`, and once at daemon start in a
low-priority, capped thread (`scanx::spawn_sweep`; no locks, no polling). Tests use an in-memory `Fs` and temp dirs only.

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

## Build cost

Many agents compile on one machine, so every session the daemon starts (`Registry::spawn`) gets a build environment from
`crates/daemon/src/buildenv.rs`, computed once at spawn from the free memory (no timer; pure functions with injected facts).
It carries `CARGO_BUILD_JOBS` (half the cores, limited by free memory, same formula as `scripts/lib/jobs.mjs`), and, only when
`sccache` is on PATH, `RUSTC_WRAPPER` plus `CARGO_INCREMENTAL=0`. A variable the user's environment or the spawn request already
sets is never overridden. Settings (`settings.json`, object `build`, additive, read by the existing `settings.get/set`; no
new method, no PROTO bump): `enabled` (true), `jobs` (null = derived), `sccache` (`auto`|`off`), `shared_target` (false),
`shared_js_cache` (true), `lock_budget_mb` (null = derived), `lock_margin_mb` (null = 2048). `shared_target` gives each project one `CARGO_TARGET_DIR`
(`<data_dir>/target/<project-id>`) and stays off because cargo's build-directory lock serializes every build of that project
across its worktrees, fingerprints include the path (workspace crates of another worktree still rebuild; only dependencies are
shared) and worktrees on other profiles thrash each other. `shared_js_cache` only points the package manager's download cache at
`<data_dir>/cache` (`npm_config_cache`, pnpm `npm_config_store_dir`, `BUN_INSTALL_CACHE_DIR`, chosen by the lockfile in the
session's folder), never the manager or the `node_modules` layout.
`axelstra lock NAME --mem-mb N` admits a holder while the declared needs fit a budget (`build.lock_budget_mb`, else physical
memory minus 2 GB): the budget is cut into 512 MB lock files (`NAME-mem-<i>.lock`), a holder takes `ceil(N/512)` free ones, a
waiter blocks in the OS call of one busy unit and rescans on its release (no polling, nobody holds units while waiting).
Admission also needs the memory available now minus the need to leave `LOCK_MARGIN_MB` (2 GB, `build.lock_margin_mb`;
pure `buildenv::mem_gate`); short memory waits for the next release, and with nobody holding it admits anyway after a warning.
So bare `lock verify/build` admits several holders when memory allows; `--slots 1` is strict one-at-a-time.
`--slots` keeps plain count slots; `lock build`/`lock verify` with neither flag declare `mem_mb_for_jobs(CARGO_BUILD_JOBS)`.
Use one mode per lock name. Lead and worker prompts (`prompts::BUILD_RULE`) and the axelstration guide tell agents to use
`axelstra lock verify`, keep the jobs value, `cargo check` before `cargo test`, `check.mjs --fast` while iterating. Two
memory gates apply to `axelstra lock`, in this order: first the governor's level gate (`wait_for_memory`, `mem.admit`: a new
build waits while the level is High or Critical, only when a daemon answers), then the memory-need gate above
(`acquire_mem`/`mem_gate`: budget and available memory). Both must pass; neither depends on the other. The governor
does not feed `Facts::free_mem_bytes` or `lock_budget_mb`.

## Git hygiene

Add paths explicitly (`git add <paths>`), never `git add -A`. `.kanban/` and `.axelstra/` are
intentionally ignored. The project is MIT-licensed (`LICENSE`); do not change the license without the owner's decision.
