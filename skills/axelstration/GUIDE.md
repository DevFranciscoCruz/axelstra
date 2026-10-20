# Axelstration

You are running inside an Axelstra terminal (as Claude Code, Codex or any other agent that can run shell commands). The `axelstra` CLI talks to the same daemon as the app, so everything you start appears as a tab in the app and the user can watch or take over.

Your own session id is `$AXELSTRA_SESSION` (PowerShell: `$env:AXELSTRA_SESSION`). A worker started by you finds you in `$AXELSTRA_PARENT`.

## Discover

- `axelstra projects` — registered projects (ids, paths, worktrees).
- `axelstra ls [--json]` — live sessions: id, worktree, agent, state (`working`, `waiting`, `idle`).
- `axelstra limit [N]` — show or set how many workers may run at once (default 6). Starting more fails with "worker limit reached": wait for one to finish.

## Isolate the work

One worktree per independent task, so workers never edit the same files:

```
axelstra worktree create <project> <name>     # prints the new worktree path
axelstra worktree rm <path>                   # when finished
```

## Delegate

Start a worker agent in a worktree. It prints the worker's session id.

```
axelstra agent <worktree> --prompt "<task, with acceptance criteria>" --report-to
```

- `--report-to` (no value) makes the worker **ephemeral**: when it calls `axelstra done`, goes idle (`--when idle`, default, with `--idle-secs`) or exits (`--when exit`), the daemon types its report into your session, closes the worker and frees its resources. You do not need to poll.
- `--agent codex` starts Codex instead of Claude.
- A worker cannot see your conversation. Give it a self-contained prompt: goal, files involved, how to verify, and the instruction below.

Put this at the end of every worker prompt:

> When finished, run `axelstra done "<summary: what you did, what is left>"` (use `--status failed` or `--status blocked` when it applies). If you need a decision, run `axelstra ask $AXELSTRA_PARENT "<question>"` and continue with the answer. If you are stuck, run `axelstra escalate "<problem>"`.

Cheap one-shot work (tests, builds, linters) does not need an agent:

```
axelstra run <worktree> --report-to -- cargo test      # output arrives in your session when it exits
axelstra terminal <worktree> --cmd "npm run dev"       # long-lived shell tab
```

## Messages between sessions

- `axelstra msg <id> "<text>"` — queue a message for session `<id>`. An agent gets a one-line notice in its screen; the full text stays in its inbox. `--quiet` skips the notice.
- `axelstra inbox [--all] [--json]` — read your messages (marks them read; `--no-mark` leaves them).
- `axelstra ask <id> "<question>" [--timeout 600]` — send a question and **block** until the other session answers. Prints the answer.
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

A task starts when everything in its `after` list succeeded, never exceeding the worker limit. `flow` prints one line per event plus the last lines of each finished task, and exits 1 if any task failed (its dependents are reported as blocked). Agent tasks end when they call `axelstra done`, go quiet (`idle_secs`, default 20) or exit. Run it in the background of your shell if you want to keep working meanwhile.

## Pattern

1. Plan: split the task into independent pieces; list them for the user if they are not trivial.
2. For each piece: `worktree create`, then `agent … --report-to` (or one `flow` file when there are dependencies).
3. Keep working or stop; reports are typed into your session as workers finish. Answer questions with `reply` and stuck workers with `msg`. Treat reports as claims to verify: run the checks yourself.
4. Integrate: review each worktree's diff, merge or cherry-pick, run the full test suite.
5. Clean up: `worktree rm` the ones you no longer need.

## Rules

- Do not start more workers than the machine can carry; the limit protects it, 3–5 at once is usually the ceiling.
- Never give two workers the same files.
- Prefer `run` for commands and `agent` only when judgement is needed.
- Look back through earlier conversations with `axelstra search "<text>"`.
