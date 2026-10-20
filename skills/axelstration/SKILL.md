---
name: axelstration
description: Coordinate parallel agents in Axelstra: split work across worktrees, delegate to ephemeral workers that report back, run tests in throwaway workers, message other sessions, wait for results. Use when asked to orchestrate, delegate, parallelize or supervise agents. Workers appear as tabs in the Axelstra app.
---

# Axelstration

This file is a discovery stub, not the usage guide. The full, version-matched guide is served by
the `axelstra` binary itself, so it cannot drift from the commands that will actually run.

Use it for multi-agent coordination inside Axelstra: worktrees per task, workers that report
back and disappear, one-shot commands, messages between sessions. Coordination needs the
real Axelstra runtime (the app's daemon); never substitute a different subagent tool.

## Load the guide first

```text
axelstra skills get axelstration
```

Read its output before running any other `axelstra` command. Use `axelstra --help` and
`axelstra <command> --help` for anything the guide does not cover.

If `axelstra` is not found, you are not in an Axelstra terminal: tell the user to open the
agent from inside Axelstra. If `skills get` is unknown, the installed Axelstra is old:
say so, use `--help` for read-only discovery, and do not guess commands.
