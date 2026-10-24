# Axelstration

You are running inside an Axelstra terminal (as Claude Code, Codex, Gemini CLI or any other agent that can run shell commands). The `axelstra` CLI talks to the same daemon as the app, so everything you start appears as a tab in the app and the user can watch or take over.

Your own session id is `$AXELSTRA_SESSION` (PowerShell: `$env:AXELSTRA_SESSION`). A worker or lead started by you finds you in `$AXELSTRA_PARENT`.

## Discover

- `axelstra projects` — registered projects (ids, paths, worktrees).
- `axelstra ls [--json]` — live sessions: id, worktree, agent, state (`working`, `waiting`, `idle`).
- `axelstra limit [N]` — show or set how many workers may run at once (default 6). Starting more fails with "worker limit reached": wait for one to finish.

## Chain of command

```
worker  ->  lead of the worktree  ->  main of the project
```

- **Main** (the session in the project's main checkout) is the dispatcher. It never edits and **never runs workers**. It creates or picks ONE worktree per theme and starts ONE **lead** agent in it. It talks only to leads.
- The **lead** owns its worktree's theme. It splits the theme into tasks, starts workers in **its own worktree**, receives their messages, reports and asks, answers them, integrates, and is the only one that talks to main (`axelstra done` / `ask` / `escalate` go to `$AXELSTRA_PARENT`).
- **Workers** talk only to their lead. `--report-to` with no value defaults to the session in `$AXELSTRA_SESSION`, so a lead's workers report to the lead.
- Reports flow up, instructions flow down. A worktree must never exist without its lead.
- The lead is the **first agent of the worktree**, whichever way it is started (Novo worktree dialog, `dispatch new`, the sidebar action "Iniciar main do worktree", or a plain `axelstra agent <worktree>` in a worktree with no lead). The daemon names it **`main`** (session meta `lead: true`; the sidebar shows it first with a star) and its task file is `main - <worktree folder>.md`. Every later agent in that worktree is a task: it is named `Tarefa N` (the counter counts only tasks, and starts at 1 again when the project has no running agent) and its `$AXELSTRA_PARENT` is the lead. The lead's own parent is the project main when one runs. Orchestration flow tasks never become leads. Sessions saved before this change come back as they were: a named `Tarefa N` is never promoted.

Before launching any agent:

1. What is the theme?
2. Does a worktree for it exist? (`axelstra dispatch list`)
3. Does it have a lead? If not, start one before anything else.
4. Am I main? Then I only start a lead (`dispatch new` / `dispatch send`), never workers.

Example, "audit the project" (5 areas: api, ui, db, build, docs): main runs `axelstra dispatch new <project> audit "Audit the project: api, ui, db, build, docs" --title "Project audit"`. One worktree `ax/audit`, one lead. The lead starts 5 workers in that same worktree (one per area, `axelstra agent <its worktree> --prompt "Audit <area>, read-only ..." --report-to`), collects the 5 reports, merges them into one answer and sends it to main with `axelstra done`. Wrong: running the 5 workers in main, or creating 5 worktrees, or creating `ax/audit` and leaving it with no agent.

## Isolate the work

One worktree per **theme**, not per task and not per worker. Workers on the same theme go into the same worktree, as several agents (`axelstra agent <worktree> ...` can be run many times on one path) that own different files; each commits only its own paths (`git add <paths>`, never `git add -A`). Open a second worktree only when the work is unrelated or two tasks must edit the same files. Read-only themes (audits, reviews) share one worktree and their workers must not edit files. Never create a separate worktree just to integrate: integrate in the theme's worktree and remove the worktrees you created when done (`worktree rm`, or `--cleanup` with `flow`). A worktree with no lead agent in it is a mistake, not a record of the work:

```
axelstra worktree create <project> <name>     # prints the new worktree path; start its lead right after
axelstra worktree rm <path>                   # when finished
```

## Delegate

Start an agent in a worktree (a lead from main, a worker from a lead). It prints the worker's session id.

```
axelstra agent <worktree> --prompt "<task, with acceptance criteria>" --report-to
```

- `--report-to` (no value) makes the worker **ephemeral**: when it calls `axelstra done`, goes idle (`--when idle`, default, with `--idle-secs`) or exits (`--when exit`), the daemon types its report into your session, closes the worker and frees its resources. You do not need to poll.
- `--agent codex` starts Codex instead of Claude, `--agent gemini` starts Gemini CLI.
- A worker cannot see your conversation. Give it a self-contained prompt: goal, files involved, how to verify, and the instruction below.

Put this at the end of every worker prompt:

> When finished, run `axelstra done "<summary: what you did, what is left>"` (use `--status failed` or `--status blocked` when it applies). If you need a decision, run `axelstra ask $AXELSTRA_PARENT "<question>"` and continue with the answer. If you are stuck, run `axelstra escalate "<problem>"`.

> Never wait silently for the human: if you need a decision, a confirmation or a permission, or you are blocked, run `axelstra ask $AXELSTRA_PARENT "<question>"` (it blocks until answered) or `axelstra escalate "<problem>"`. If the choice is reversible, decide, say so in your report and carry on.

Cheap one-shot work (tests, builds, linters) does not need an agent:

```
axelstra run <worktree> --report-to -- cargo test      # output arrives in your session when it exits
axelstra terminal <worktree> --cmd "npm run dev"       # long-lived shell tab
```

## Messages between sessions

- `axelstra msg <id> "<text>"` — queue a message for session `<id>`. An agent gets a one-line notice in its screen; the full text stays in its inbox. `--quiet` skips the notice.
- `axelstra inbox [--all] [--json]` — read your messages (marks them read; `--no-mark` leaves them).
- `axelstra ask <id> "<question>" [--timeout 600]` — send a question and **block** until the other session answers. Prints the answer.
- `--thread NAME` on `msg` and `ask` groups related messages (replies inherit it); `axelstra thread NAME` prints the whole conversation in order.
- `axelstra reply <message-id> "<answer>"` — answer a question you were notified about (the number is in the notice). This unblocks the asker.
- `axelstra done "<summary>" [--status ok|failed|blocked]` — worker: report to the parent and finish.
- `axelstra escalate "<problem>"` — worker: ask the parent for help. The worker shows as **waiting** (red dot in the app, `waiting` in `ls`) until you answer with `axelstra msg <worker> "…"`.
- `axelstra say <id> "<text>"` — paste text straight into a session and press Enter (use it to redirect an agent right now; prefer `msg` for anything it can read later).

## Follow up

- `axelstra read <id> [--lines N]` — what a session printed (ANSI stripped).
- `axelstra wait <id> [--idle-secs 3] [--timeout 600]` — block until it is quiet or has exited (exit code 2 on timeout).
- `axelstra kill <id>` — stop a session.

## Task graphs

For work with dependencies, write a JSON file and let `flow` run it:

```json
{"tasks": [
  {"id": "api",   "worktree": "C:/repo-api", "agent": "claude", "prompt": "Add the endpoint ... When finished run `axelstra done`."},
  {"id": "ui",    "worktree": "C:/repo-ui",  "agent": "claude", "prompt": "Build the page ..."},
  {"id": "tests", "worktree": "C:/repo",     "cmd": "cargo test", "after": ["api", "ui"]}
]}
```

```
axelstra flow plan.json [--timeout 3600]
```

If the app was closed in the middle of a plan, run the same command again with `--resume`: tasks that already finished are skipped and the rest runs again. While a plan runs, the app shows the worktrees still waiting for their turn as hollow dots.

Add `--cleanup` to remove each task's worktree as soon as the task succeeded. A worktree is only removed when it is clean and all its commits are on a remote (otherwise `flow` says why it kept it), never the main one, and never while another task still uses it.

A task starts when everything in its `after` list succeeded, never exceeding the worker limit. `flow` prints one line per event plus the last lines of each finished task, and exits 1 if any task failed (its dependents are reported as blocked). Agent tasks end when they call `axelstra done`, go quiet (`idle_secs`, default 20) or exit. Run it in the background of your shell if you want to keep working meanwhile.

## Parallel workers, heavy commands in turns

Prefer running workers **at the same time**: tasks in a `flow` file without `after` start together (up to the worker limit). Do not chain them just because a command at the end is heavy. Let the workers run side by side and make only the heavy command take turns:

```
axelstra lock verify -- bun run verify          # one verify at a time, across every session
axelstra lock build --slots 2 -- cargo build    # two at a time
```

`lock` waits for a free slot, runs the command, passes its exit code through, and frees the slot when the command ends (or the process dies). Tell every worker to run its full verify, build or test suite through `axelstra lock`.

## Seeing and driving the desktop

For desktop apps that have no CLI or API (Windows and macOS). The user must turn it on first (command palette, "Controlo do ecrã"); until then every command fails and you cannot enable it yourself. Ask the user if you need it.

- `axelstra computer windows [--json]` — visible windows: id, process, size, position, title.
- `axelstra computer screenshot [--window ID] [--out file.png]` — saves a PNG (whole screen, or one window) and prints its path: read the image to look at it.
- `axelstra computer click X Y [--window ID] [--button left|right|middle] [--double]` — coordinates are pixels in the screenshot (from the window's top-left with `--window`).
- `axelstra computer type "text" [--window ID]`, `axelstra computer key ctrl+s [--window ID]`, `axelstra computer scroll -3 [--x X --y Y]`, `axelstra computer focus ID`.

Take a screenshot before and after acting, and prefer a CLI or API when the app has one.

## Pattern

1. Plan: split the task into independent pieces; list them for the user if they are not trivial.
2. Main: `dispatch new` (worktree + lead). Lead: for each piece, `agent <own worktree> … --report-to` (or one `flow` file when there are dependencies).
3. Keep working or stop; reports are typed into your session as workers finish. Answer questions with `reply` and stuck workers with `msg`. Treat reports as claims to verify: run the checks yourself.
4. Integrate: review each worktree's diff, merge or cherry-pick, run the full test suite.
5. Clean up: finished leads close on `done` and the sweep removes integrated worktrees by itself; `worktree rm` only the ones it cannot judge.

## Rules

- Do not start more workers than the machine can carry; the limit protects it, 3–5 at once is usually the ceiling.
- Never give two workers the same files (same worktree is fine when their files are disjoint).
- Prefer `run` for commands and `agent` only when judgement is needed.
- Look back through earlier conversations with `axelstra search "<text>"`.

## Dispatcher mode (main session)

When the user works from the **main** worktree you are a dispatcher, not an implementer. Do not edit code there and do not start workers there. Every request belongs to a theme; every theme has one worktree and one lead agent. You start leads, nothing else.

For each new request:

1. `axelstra dispatch list --json` — open worktrees with `title`, `areas` (folders they touch), `last_request`, running `agents`, and `overlaps_with` (worktrees editing the same files).
2. Decide where the request belongs, using the evidence: the same system or feature, the same folders in `areas`, a follow-up to `last_request`. A shared word is not enough.
3. Act:
   - **Clear match** → `axelstra dispatch send <branch-or-path> "<request>"`. It goes to that worktree's lead (started if none runs). Tell the user in one line: `sent to <branch>`.
   - **No match** → `axelstra dispatch new <project> <short-name> "<request>" --title "<3-6 word topic>"`. It creates the worktree and its lead together. One theme per worktree: a different system or an unrelated feature is a new theme. Never use `worktree create` alone.
   - **Unsure** (two plausible worktrees, or a request that mixes themes) → ask the user which one, in one short question. Never create a worktree just to avoid asking.
4. If the request touches files another worktree is already editing (`overlaps_with`, or its `areas`), say so before sending and let the user choose.
5. Never send work to a worktree marked merged or archived (`list` hides them; `send` refuses). Open a new theme instead. When a theme is finished: `axelstra dispatch archive <branch>`.
6. Leads report to you with `done`/`ask`/`escalate`. Answer with `reply`/`msg`; verify claims before telling the user it is done.
   When a lead asks (or the stall watchdog reports one quiet), answer or decide yourself; if the decision is the user's, put it to them in one short message with your recommendation, then relay the answer. Never leave a lead waiting.

- `axelstra dispatch title <worktree> "<title>"` — rename a theme.
- Keep replies short: the user wants to see where each request went, not an essay.

## Inside a worktree (not main)

You are the lead of this worktree's theme. Own it end to end:

- Split the theme into tasks and start workers in **this** worktree (`axelstra agent . --prompt ... --report-to`); do not create more worktrees for subtasks. Their sessions appear inside this worktree's box in the app.
- Workers report to you; answer their asks (`reply`), unblock escalations (`msg`), verify their reports, integrate their work.
- Talk to main only through `axelstra done` / `ask` / `escalate` (they go to `$AXELSTRA_PARENT`). Workers never talk to main.
- **Never wait silently for the human.** Writing "waiting for your decision" in the terminal tells nobody. Any decision or blocker goes up with `axelstra ask $AXELSTRA_PARENT "<question>"` (blocks until main answers) or `axelstra escalate "<problem>"`; a reversible choice you make yourself, noting it in the report. The same rule goes to every worker (see its prompt above).
- **Stall watchdog.** An agent quiet for 5 minutes (Settings, "Agente parado") without `done`/`escalate` and with no running children is marked **stalled** (orange hourglass dot in the sidebar, never green), its parent gets an automatic message with the last screen line, and the app notifies. When you get that message, answer the worker (`axelstra msg <id> "…"`) or take over.
- Work small enough to do yourself needs no worker.
- **Lifecycle.** A lead started by `dispatch new/send` is ephemeral like a `--report-to` worker: when it calls `axelstra done "<report>"` its session closes by itself, and the report is typed into main and kept in the task file. Finish with `done` only when everything is committed and verified. Main only has to read the report; it does not close leads or remove worktrees.
- **Cleanup.** The janitor sweep (every 10 minutes, and a few seconds after every `done` or merge) removes a worktree and its branch when the branch is inside the default branch or inside another live `ax/*` branch (stacked integration). It never touches a worktree with uncommitted changes, with a session that has not reported (a session that called `done` and is quiet does not count as busy), or with unmerged work, and it never deletes a branch whose commits no other ref holds. Every removal shows a toast and a notification-center entry ("worktree X removido (integrado em Y)"). `axelstra worktree sweep [--dry-run]` runs it by hand.
