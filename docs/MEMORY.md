# Memory governor

Axelstra runs many agents, builds and test suites at once. When the machine runs low on memory the whole desktop slows
down. The governor watches free memory, tells you, delays new work and, as a last resort, freezes the biggest consumers
until memory recovers. Contributor notes (code layout, event sources, locks): the "Memory governor" section of
[`AGENTS.md`](../AGENTS.md).

## What it does

Three pressure levels, from the machine's free memory (memory a new process could get without paging: Windows
available physical memory, Linux `MemAvailable`, macOS free + inactive + speculative + purgeable pages):

| Level | Entered when free memory is at or below | What happens |
| --- | --- | --- |
| normal | (default) | nothing |
| high | 20% of the total **or** 3072 MB | a notice; new agents started by agents are refused; the worker limit is halved; new `axelstra lock` builds wait |
| critical | 10% of the total **or** 1536 MB | all of the above; the worker limit is 1; agents and builds are frozen one at a time until the level drops |

- **Hysteresis.** A level is left only when free memory is above both lines plus a margin (5% and 512 MB), so a machine
  hovering at a line does not flap. The margin is fixed.
- **Admission.** At high and critical: an agent started by another agent is refused ("not started: memory is ...");
  a worker is refused once the lowered limit is reached (`axelstra limit N` is the normal limit: high allows half of it,
  at least 1; critical allows 1). Never refused: terminals, and anything you start yourself from the app or the CLI.
  `axelstra lock` (so every build and test that goes through `axelstra lock verify`) waits **before** taking its slot,
  prints `waiting: low memory`, and goes on the moment memory recovers. It gives up only after its usual `--timeout`.
  Builds that are already running are not touched by this.
- **Pausing (critical only).** One victim at a time, in this order, until the level drops: 1. builds and tests running
  under `axelstra lock` (biggest first); 2. idle workers (quiet for 20 s); 3. other agents (quiet for 1 min); 4. leads;
  5. the project's main agent (quiet for 5 min). Never paused: the daemon, **terminals**, a session you typed into in the
  last 2 minutes, and anything holding less than 32 MB. Builds go first because nobody talks to them; a frozen agent's
  network calls (the model API, git, MCP) may time out while it is frozen and fail when it continues.
- **Pausing does not free memory.** A frozen process keeps its memory; it just stops growing, and the OS can page it
  out. Expect relief to be gradual, not instant.
- **Resuming.** Frozen items continue by themselves when the level leaves critical. You can resume any of them sooner
  (panel, `axelstra resume`). Something you paused by hand stays paused until you resume it.

Nothing stays frozen behind a stopped daemon: on a normal exit, on a panic and on the next start after a crash the
daemon continues everything it froze (after a crash, only processes that are still the same ones). Killing or removing a
session resumes it first.

## In the app

- **Status bar chip:** free memory and level colour; it also shows how many items are paused. Click it for the panel.
- **Memory panel:** level, free and total memory, the sessions using the most memory, what is paused (with the reason,
  `low memory` or `manual`, and *Resume* / *Resume all*), and a link to the settings.
- **Sidebar:** a paused session shows a marker; its menu has *Pause* / *Resume*.
- **Notifications:** a toast and a bell entry when the level rises to high or critical or when sessions are frozen; a
  toast when memory is normal again and when sessions continue.
- **Settings > Resource governor:** the switches and the four lines below.

## Settings

Saved in `settings.json` under `governor` (change them in the app, or `mem.config`). Defaults:

| Key | Default | Meaning |
| --- | --- | --- |
| `enabled` | `true` | off: the level stays normal and nothing is announced, delayed or paused |
| `admission` | `true` | refuse and delay new work at high and critical |
| `auto_pause` | `true` | freeze victims at critical |
| `warn_only` | `false` | say what would be done and do nothing (see below) |
| `high_free_pct` / `crit_free_pct` | `20` / `10` | percentage of total memory (1..90) |
| `high_free_mb` / `crit_free_mb` | `3072` / `1536` | megabytes (1..4194304) |

A level starts at the percentage **or** the megabytes, whichever is hit first, so the megabyte lines rule on small
machines and the percentages on big ones. The critical values must stay below the high ones. A bad value is rejected;
a damaged `governor` key falls back to the defaults field by field.

### Warn-only mode

Set `warn_only` to try the governor without consequences. Levels, notices and `axelstra mem` work as usual, but nothing
is refused or waits (`axelstra lock` goes on), and at critical the daemon only broadcasts a `memory.would_pause` event
naming the sessions and builds it would have frozen. Manual pause and resume keep working. Run a day in this mode if you
are unsure about the thresholds for your machine.

## Command line

```
axelstra mem [--json]       level, free/total memory, thresholds, the biggest sessions, what is paused
axelstra resume <session>   continue one paused session (and the builds it runs)
axelstra resume --all       continue everything that is paused
```

`axelstra lock NAME -- cmd` waits for memory by itself (see Admission). A user can pause a session from the app
(sidebar menu or the panel); agents cannot pause, resume or reconfigure (`mem.config`, `mem.pause`, `mem.resume` are
refused to them on purpose, so an agent cannot switch the safety net off).

## Troubleshooting

- **A build says `waiting: low memory` and nothing moves.** `axelstra mem` shows the level and the biggest sessions.
  Close something big, or resume/kill what you do not need. To go on now, set `admission` to `false` (or `warn_only`).
  The wait ends by itself when memory recovers; `axelstra lock --timeout` bounds it.
- **An agent was refused with "not started: memory is high".** Start it yourself from the app, or wait for the level to
  drop. Workers are refused only past the lowered limit.
- **A session looks hung.** Check for a paused marker in the sidebar or `axelstra mem`. `axelstra resume --all` fixes it.
  A frozen agent that was calling a network service may report a timeout once it continues; ask it to retry.
- **It froze something it should not, or too early.** Raise the headroom the other way: lower `crit_free_pct` /
  `crit_free_mb`, turn `auto_pause` off (admission and notices stay), or use `warn_only`.
- **The numbers look off.** `free_mb` is "available", not "unused": cached memory the OS can drop counts as free.
- **The chip is missing.** The running daemon is older than the app (no governor). Restart Axelstra.
- **No pressure events on Linux.** The kernel may refuse the PSI trigger to an unprivileged process or lack PSI; the
  governor then samples (stderr: `governor: PSI unavailable`). It still works, with a slower reaction.
- **Stuck processes after a crash.** The next daemon start continues them. Manually, on Unix: `kill -CONT <pid>`;
  `paused.json` in the data dir lists the pids that were frozen.

## How it reacts without polling

There is no fixed timer. The daemon waits for the OS's own memory signal (Windows memory resource notifications, Linux
PSI). Those fire close to exhaustion, later than the lines above, so the wait also carries one computed deadline: far
from the lines it is long (up to 60 s), near them or while the level is not normal it is short (1 to 10 s). macOS has no
such signal, and neither does any machine where it cannot be set up: there the computed deadline is the only wake (up
to 15 s when memory is plentiful). So the worst-case reaction to memory being eaten very fast is about a second on
Windows and Linux when close to a line, and not guaranteed to be faster than the deadline on macOS.

## Verification status, honestly

| | Windows | Linux | macOS |
| --- | --- | --- | --- |
| Memory reading | developed here, no pressure test | unit-tested parser; CI | unit-tested `vm_stat` parser; CI |
| OS pressure event | memory resource notifications, developed here; not exercised under real pressure | PSI trigger: **not exercised under real memory pressure** | none (sampler) |
| Freezing | `NtSuspendProcess`, real cycle tested on a child process the test owns | `SIGSTOP`/`SIGCONT`, same test runs in CI | same as Linux, CI only |
| Hand-tested in the app | the development platform | **no** | **no** |

What this means:

- Everything above was developed on Windows. Linux and macOS builds run the same tests in CI (the state
  machine, admission, victim choice and the real freeze/continue of a child process are cross-platform), but nobody has
  watched the PSI event fire on a loaded Linux machine, nor the sampler on a loaded Mac, nor the app on either.
- No test ever pushes a real machine into memory pressure. The levels are driven by a fake memory source.
- The freeze of a whole process tree is verified on a simple child. Programs that watch their own heartbeat (some
  Electron apps, databases, remote sessions) may notice when they continue.
- If you run it on Linux or macOS, start with `warn_only` and report what you see.
