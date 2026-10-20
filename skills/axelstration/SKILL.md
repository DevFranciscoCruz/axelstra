---
name: axelstration
description: Coordinate parallel agents inside Axelstra with the `axelstra` CLI. Use when asked to split work across worktrees or agents, delegate to workers, run tests in a throwaway worker, or collect results from other sessions. Workers show up as tabs in the Axelstra app.
---

# Axelstration

You are running inside an Axelstra terminal. The `axelstra` CLI talks to the same daemon as the app, so everything you start appears as a tab in the app and the user can watch or take over.

Your own session id is `$AXELSTRA_SESSION` (PowerShell: `$env:AXELSTRA_SESSION`). Workers report back to it.

## Discover

- `axelstra projects` — registered projects (ids, paths, worktrees).
- `axelstra ls [--json]` — live sessions: id, worktree, agent, state (`working`, `waiting`, `idle`).

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

- `--report-to` (no value) makes the worker **ephemeral**: when it goes idle (`--when idle`, default, with `--idle-secs`) or exits (`--when exit`), the daemon types its last output (`--lines`) into your session, closes the worker and frees its resources. You do not need to poll.
- `--agent codex` starts Codex instead of Claude.
- Give each worker a self-contained prompt: goal, files involved, how to verify, and "finish by summarising what you did and what is left". It cannot see your conversation.

Cheap one-shot work (tests, builds, linters) does not need an agent:

```
axelstra run <worktree> --report-to -- cargo test      # output arrives in your session when it exits
axelstra terminal <worktree> --cmd "npm run dev"       # long-lived shell tab
```

## Follow up

- `axelstra read <id> [--lines N]` — what a session printed (ANSI stripped).
- `axelstra wait <id> [--idle-secs 3] [--timeout 600]` — block until it is quiet or has exited (exit code 2 on timeout).
- `axelstra say <id> "<message>"` — message an agent session: pasted as one block, then Enter. Use it to answer a worker's question or to redirect it.
- `axelstra kill <id>` — stop a session.

A worker that needs a decision stops and shows as **waiting** in `axelstra ls` (and as a red dot in the app). Read it, then answer with `say`.

## Pattern

1. Plan: split the task into independent pieces; list them for the user if they are not trivial.
2. For each piece: `worktree create`, then `agent … --report-to`.
3. Keep working or stop; results are typed into your session as workers finish. Treat each as a report to verify, not as truth: run the checks yourself.
4. Integrate: review each worktree's diff, merge or cherry-pick, run the full test suite.
5. Clean up: `worktree rm` the ones you no longer need.

## Rules

- Do not start more workers than the machine can carry; 3–5 at once is usually the ceiling.
- Never give two workers the same files.
- Prefer `run` for commands and `agent` only when judgement is needed.
- Look back through earlier conversations with `axelstra search "<text>"`.
