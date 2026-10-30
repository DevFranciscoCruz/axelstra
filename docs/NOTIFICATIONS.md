# Notifications

Who tells whom what, and why. One section per layer: daemon, host (Tauri), frontend.

## Daemon

Principles: a notification is an **event** (never read off screen text), goes to **one** recipient, and exists
only when somebody has something to do. A parent agent hears about a child when the child reports (`done`,
`escalate`, `ask`), waits for the user (hook event) or is truly stalled, never because it is merely working.

### Inventory of daemon-side sources

| Source | Origin | Fires when | Recipient | Action needed? | Typical volume | Decision |
|---|---|---|---|---|---|---|
| Stop-hook nudge, to the agent itself | the agent's own `Stop` (Claude/Codex) / `AfterAgent` (Gemini) hook -> `session.hint {state: idle, event: "stop"}`; for an agent with no hooks, its ephemeral reaper after 120 s quiet | An Axelstra-managed agent (parent agent, no live child) ends its turn without `done`, `await`, a pending `ask`/`escalate`/wait, and is not user-driven | The agent (`prompts::STOP_NUDGE`, pasted into its terminal, not a bus message): "[axelstra] Your turn ended without done, await or ask. Finish with `axelstra done "<report>"`, `axelstra await --reason "..."` or `axelstra ask $AXELSTRA_PARENT "<question>"`." | Yes: finish with `done`, `await` or `ask` | At most 1 per stop episode and 1 per session per 10 min | **New**. The episode resets only on real new work (user input, an announced bus message, a working hint not caused by the nudge). An old CLI sends no `event`, so no nudge |
| Stall, to the parent | `stall_tick` (`server/loops.rs`), text `stall::parent_text` | The session was nudged (or has no Stop hook) and stayed quiet for `stall_minutes` (default 15) after that, and none of the legitimately quiet states holds (see below) | Parent agent (a `notice` typed into its TUI + inbox, one line from `stall::parent_text`: `"<label>" (session N) stopped without done/await/ask and ignored one reminder. Check it: axelstra read N`; or "never reported a turn (a trust dialog or a lost prompt?) and has been quiet M min" / "stopped without done/await/ask and has been quiet M min") | Maybe: look, nudge or take over | At most 1 per stall episode; several children in one pass are **aggregated** into one line | **Keep, narrowed**; never repeated while the state is unchanged (a stray output byte does not start a new episode) |
| Stall, to the user | `session.list` field `stalled` (+ `stall`), `session.changed` broadcast | Same stall, only for a session with no parent agent that is not user-driven; once per episode. A child its parent ignores is never escalated to the user | Human (red/stopped state, notice in the app) | Yes | 0 normally | **Keep, gated**. New additive field `stall_parent`: stalled but a parent agent is handling it (the app should not alert) |
| Waiting (permission, question) | Agent hook, `axelstra hook claude\|gemini\|codex` -> `session.hint` -> `Hints` (`core/waiting.rs`); `ask`/`escalate` -> `Bus::pending_of` | Claude `permission_prompt`/`elicitation_dialog`, Gemini `ToolPermission`, Codex `PermissionRequest`; `axelstra ask`/`escalate` | Human (red dot, bell, OS notification); parent agent also told once per wait (`tell_parent_waiting`, a `notice`) | Yes | One per real prompt | **Keep**. `idle_prompt` and any unknown notification type are **not** a wait |
| Idle (`idle_prompt`, Stop) | same hook | The agent finished its turn | nobody (a managed agent may get the Stop-hook nudge above) | No | - | **Silence** (only state) |
| `done` | `axelstra done` -> `msg.send kind=done` | A worker/lead reports | Parent (notice + inbox), task file | Yes, read the report | One per task | **Keep** |
| `escalate` / `ask` | `axelstra escalate/ask` | An agent is blocked | Parent (notice); counts as waiting for the user | Yes | Rare | **Keep** |
| Worker reaper report | `ReaperTask::finish` (`server/loops.rs`) | An ephemeral worker's process ends, or a plain command is closed for being quiet (an agent is never closed or reported finished on silence: only `done` or its exit) | Parent | Yes | One per worker | **Keep**; `IDLE_NUDGE` is gone (an agent without hooks gets the stop reminder instead) |
| `janitor.events` | `janitor.rs`, `orchend.rs` | A worktree was removed (`integrated`, `empty`), or could not be integrated (`attention`) | Human (toast + notification center) | `attention`: yes; the rest: informative | A few per day | **Keep** `attention`; removals are candidates to **aggregate** in the app (frontend review) |
| `integrate::attention` | `integrate.rs`, shown in `worktree.overview` | Automatic integration did not happen (conflict, dirty checkout, default branch moved) | Human (sidebar "por integrar" + Integrar button) | Yes | Rare | **Keep** (state, not a push) |
| `orch.kept` | `orchend::finish` | A failed/partial Kanban run left work on `orch/<run>` | Human (janitor event) | Yes: review or retry | One per bad run | **Keep** |
| `budget.events` | `budget.rs` | A worktree crosses its budget level | Human (toast + notification center); agents are paused only with the opt-in | Yes at `Over` | Rare | **Keep** |
| `triggers.events` | `server/triggers.rs`, `prwatch.rs` | CI-fix trigger fired / PR state changed | Human (toast + notification center) | Informative | Per CI failure | **Keep**; fires on events, not a timer |
| `notices.take` | `store::notice`/`notice_coded` | State was lost or recovered (corrupt file moved to `.bad.<ts>`, migration) | Human (toast on next poll) | Informative | Rare | **Keep** |
| `session.exited` | `Registry::retire` | A session ended by itself | Human (toast only for a non-zero exit code of a terminal; agents get the usual done/stopped) | Only on failure | One per exit | **Keep** |

### Root causes found (false alerts)

1. **The stall check only knew four facts** (reported, escalated, awaiting, live children) plus screen silence.
   It did not know that the agent's process tree had a background shell or a build (`axelstra lock verify`,
   `axelstra wait`, a CI watcher), that it was waiting for a worker slot (`session.worker_limit` is a refusal, the
   agent retries by itself), or that it had just received a message. Hence "stalled for 5 min ... Last screen
   line: ⏵⏵ auto mode on" while the lead had `check.mjs` running. Fixed in `stall.rs` / `stall_facts`:
   `bg_work` (see below), `slot_wait`, `since_msg_ms`.
2. **Automatic parent messages were `kind: "message"`** (`stall_tick`, `Registry::tell_parent_waiting`). In
   `Bus::send` a `message` to a session clears its escalation, its pending `ask` and **its `done` report**
   (`msgbus.rs`, arm `"reply" | "message"`). A watchdog or waiting notice therefore wiped the parent's own state: a
   lead that had reported `done` became "not reported" and was flagged stalled, a pending question vanished and
   reappeared. They are now `kind: "notice"`, which changes no state (test `a_notice_changes_no_state_of_its_receiver`).
3. **The user was alerted for a stall the parent was already told about** (`session.list` `stalled` was the plain
   stall). Now `stalled` means "needs a person" (`stall::needs_human`); `stall_parent` says a parent agent has it.
4. **Noise in the text**: the last screen line was a status bar or a bare prompt. `stall::last_line` now skips lines
   that do not start with a letter/digit or have fewer than 8 alphanumerics, and the message names the probable cause.
5. **The watchdog never broadcast**: a stall flipping only showed up on the next unrelated list refresh. It now
   broadcasts `session.changed` when the set of sessions needing the user changes.

Waiting audit (no false positive found in the daemon): `idle_prompt` becomes `idle_input` and `Hints::set` turns it
into Idle; an unknown or missing `notification_type` says nothing (`hook.rs`); a hint clears on Stop/UserPromptSubmit
(Idle/Working replace it), on typing, on output after a 1.5 s redraw slack, after `done` and after 2 h
(`waiting.rs` tests `a_wait_is_cleared_by_stop_prompt_submit_input_and_output`, `a_wait_needs_a_kind_and_idle_input_is_not_one`).
A hint is accepted only from the agent's own session (`require_own`). If the user still sees a "question" with no
question, the remaining candidates are outside the daemon: the app turning `stalled` into a "stopped" notice
(fixed at the source by 3), or a stale `axelstra ask` whose asker was interrupted (`Bus::pending` lasts until a reply
or `done`; not changed, see "Not verified").

### Legitimately quiet: never stalled, never a notice

No reminder, parent line or user alert while any of these holds: `awaiting` (`axelstra await`); a pending wait (hook
permission/question, `ask`/`escalate` pending); hibernated; paused (`mem.pause`); a blank agent that never got input or
a message (`stall::never_asked`, no longer only for leads); a user-driven session (no parent agent and the user typed
into it: the user is the driver); a session with live children; a session that reported `done`. The default threshold
is 15 min (`stall_minutes`, 0 = off).

### How "legit work" is decided (state, not screen text)

`stall::Facts` -> `is_stalled` is false when any of these holds:

- `awaiting`: `axelstra await` active (unchanged).
- `live_children`: a child session of it is running and not reported (unchanged).
- `bg_work`: a process below the session's root, **not part of its start-up** (`stall::Work`): descendants seen in
  the first 60 s after the session was first observed are its baseline (the agent's MCP servers and helpers); any
  later one alive counts as work. This covers a background shell, a build or test, and `axelstra lock ...` both
  holding and waiting for a lock (the CLI process is a descendant while it waits). There is no OS event for a
  grandchild exiting, so while only `bg_work` explains the silence the watchdog rechecks every 60 s
  (`BG_RECHECK_MS`, a documented exception like the other OS-resource sleeps in `AGENTS.md`).
- `slot_wait`: the session's `session.spawn` was refused with `session.worker_limit` and the limit is still reached
  (cleared when it spawns, when a slot frees, or after 2 h). A worker ending bumps the pulse, so no timer.
- recent message: `quiet_ms = min(idle, since input, since inbox message)`.
- `reported`, `escalated`, not running, plain terminal, project main agent (unchanged).

There is no "hibernated" state in the daemon (a frozen/hibernating machine only jumps the clock; the existing
wake-up logic tolerates it), so nothing to add.

Episodes (`stall::Turns`): the parent is told once per episode; the episode ends only on real new work (the user
typed, a message reached its inbox, a working hint that is not the reminder's own turn) or when the session leaves.
A stray output byte or a redraw does not end it. The watchdog sleeps until the pulse or one computed moment
(`stall::next_check_ms`).

Coordinators are not told about state: they pull it (`axelstra ls` quiet-for and next action, `--json`
`quiet_since`/`liveness`/`next_action`) and block on `axelstra inbox --wait [--types ...]` until a report or a pendency
arrives.

### Not verified / left

- The process baseline is a heuristic: an MCP server that first starts later than 60 s after the agent started
  is treated as work (the watchdog stays quiet, the safe direction). Not measured against a real Claude/Gemini/Codex
  on all three platforms.
- A stale `ask` pending after the asking agent was interrupted still shows as waiting until a reply or `done`.
- The frontend must read `stall_parent` (additive; no PROTO bump) and keep alerting only on `stalled`; that is task 2/3.

## Host (Tauri)

The host adds no notification of its own: it only delivers what the frontend asks for. Nothing in `app/src-tauri/src`
fires on its own account (no tray, no badge, no dock count).

| Origem | Quando dispara | Quem recebe | Ação necessária? | Volume típico | Decisão |
|---|---|---|---|---|---|
| `notify_session` (`main.rs`): Windows toast with click-through, `MessageBeep` | Only when the frontend calls it (see Frontend) | Human | Depends on the caller; the frontend now calls it only for "Preciso de ti" and for categories the user switched on | Was one per agent that went quiet; now a few per day | **Manter**; gated by the frontend category and do-not-disturb before the call; `notify_mode` (`all`/`waiting`/`off`) and `notify_sound` stay as a global ceiling |
| `request_user_attention` (taskbar flash) inside `notify_session` | Same call | Human | Same | Same | **Manter** (flashes only when a toast was really requested) |
| `@tauri-apps/plugin-notification` fallback in `store/sessions.ts` | Non-Windows, or a toast without click callback | Human | Same | Same | **Manter** (same gate) |
| `notification-click` event | The user clicks a toast | App (opens the session) | n/a | n/a | **Manter** |
| Native dialogs (`native.rs`) | The user's own action | Human | n/a | n/a | Not a notification |

## Frontend

Principle: the human is interrupted (system notification, sound, toast, bell counter) only when an action is needed.
Everything else is state (dots, sidebar) or a quiet entry in the notification centre.

### Categories and defaults

| Category | Label (pt-PT) | Kinds | Default | Contains |
|---|---|---|---|---|
| `need` | Preciso de ti | `waiting`, `attention` | **on** | permission/question from an agent's hook, `ask`/`escalate`, a stall nobody handles (`stalled`), a janitor `attention` (integration conflict) |
| `done` | Concluído | `done` | off | a top-level agent whose process ended |
| `info` | Informativo | `merged`, `ci-pass`, `budget` (warn), `done` of a session that has a parent agent, trigger `ci_fix` | off | worktree removals/integrations, CI passing, budget warning, automatic CI fix |
| `error` | Erros | `error`, `ci-fail` | **on** | failed actions, a terminal exit code != 0, budget exceeded, trigger that could not act |

A category that is **off** is still recorded in the centre, but already read: no system notification, sound, toast or
bell counter. **Não incomodar** (do not disturb) records every enabled category as unread but suppresses the system
notification, sound and event toasts (janitor, trigger, budget); "Preciso de ti" keeps the red dot and the sidebar
state (state is not a notification). Failures of the user's own actions (`setError`) still toast, being the answer to
what they just did. Preferences: `axelstra.notifyPrefs` (localStorage, mirrored to the daemon's settings `ui`; no new
daemon method, no PROTO change). Policy in `app/src/notifyPrefs.ts`; recording in `notifications.ts` (`push`).

### Inventory

| Origem | Quando dispara | Quem recebe | Ação necessária? | Volume típico | Decisão |
|---|---|---|---|---|---|
| `store/sessions.ts`: `stalled` + `stall` | The daemon says nobody handles the stall (no live agent parent, or the parent did not resolve it) | Human: centre entry (`need`, episode key `stall:<id>`), system notification if `need` may interrupt | Yes | ~0 | **Manter**, one per episode |
| `stall_parent` (session.list) | A parent agent was told | Nobody in the app | No | many | **Silenciar**: ignored (type added in `api.ts`; `dotOfSession` and trackers read only `stalled`). No sidebar hint added |
| `store/sessions.ts`: `wait` (hook/ask/escalate) via `waitNotice.ts` | An explicit wait event, once per episode | Human: centre entry + system notification (`need`) | Yes | One per prompt | **Manter** |
| `store/sessions.ts`: agent working then idle ("stopped") | A finished turn | Nobody | No | Was one per turn: the main false alarm | **Remover**: state only (sidebar dot / attention mark). `L.notify.stopped` deleted |
| `store/sessions.ts`: agent process ended ("done") | `running` turned false | Human if `done` is on (category `info` when the session has a parent); centre entry | Rarely | Per task | **Agregar** ("N sessões concluíram") and off by default |
| `session.exited` non-zero code | A terminal/agent ended with an error | Human: error toast + centre | Yes | Rare | **Manter** |
| `janitorEvents.ts` removals | Worktree integrated/empty removed | Centre entry (aggregated); one toast per batch if `info` on | No | A few per day | **Agregar**, silent by default |
| `janitorEvents.ts` attention | Integration needs a person | Toast + centre (`need`) | Yes | Rare | **Manter** |
| `triggerEvents.ts` | CI-fix trigger fired / refused | `ci_fix`: `info` (silent); refusal: `error` | Refusal yes | Per CI failure | **Silenciar** the self-started fix, **manter** the refusal; one entry per sha |
| `budgetEvents.ts` | Budget level crossed | warn: `info` (silent); over: `error` toast + centre | Over yes | Rare | **Manter** over, **silenciar** warn |
| `store/memory.ts`: `memory.pressure`, `session.paused`/`resumed` | Memory level changed; the governor froze or resumed sessions | Human: centre entry (`attention`) | Critical and a freeze: yes | Rare | **Manter** critical and freeze as `error` (toast only if `error` may interrupt), high and the way back as `info` (silent by default); episode keys `memory:level:<level>`, `memory:paused`; no new category |
| `toasts.ts` error/warn sticky | `setError`, any `toast` of kind error or sticky warn | Human toast; also recorded (`error`/`attention`) | Yes | Per failure | **Manter**; recorded read when the category is off |
| `notifications.ts` `shipEvents` | PR merged / CI changed (polled status) | Centre (`info`/`error`) | CI fail yes | Per PR | **Manter**, categorised |
| `NotificationBell.tsx` | Unread count in the sidebar header | Human | n/a | n/a | **Manter**; counts only unread (enabled categories) |
| `UpdateToast.tsx`, `VersionSkew.tsx` | App/daemon update | Human | Yes (choose) | Rare | Not an agent notification, unchanged |
| Document title, badge, tray, sounds | - | - | - | - | None exist; the only sound is the host beep in `notify_session` |

### Aggregation and dedupe (`notifications.ts`)

- A `done` or `merged` entry that follows another of its kind within 10 min (`AGGREGATE_MS`), same read state, folds into
  it: "3 sessões concluíram a tarefa" (`n` counts them; worktree/session kept only when shared).
- An identical entry (kind, worktree, text) within 2 min is dropped; an entry with an episode `key` (stall, trigger sha,
  budget level) is the same notification for 30 min (`EPISODE_MS`).
- Things that need a person (`waiting`, `attention`) and errors are never folded.

### Not verified

- No run of the real app (OS toasts, Windows focus/DND) here; covered by vitest with the Tauri `invoke` mocked.
- `notify_mode` and the new categories overlap (the mode is the host-level ceiling); not merged to avoid migrating a daemon setting.
