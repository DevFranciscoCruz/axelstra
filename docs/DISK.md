# Disk hygiene

Worktrees in `.axelstra-wt` grow to 5-10 GB each, almost all of it regenerable build output (cargo `target/`,
`node_modules/`). The janitor removes a worktree once its work is integrated; a worktree that still holds work stays, and so
did its builds. Disk hygiene takes the build output of worktrees nobody uses, and never the worktree itself.

Code: `crates/daemon/src/artifacts.rs` (what is reclaimable, the rules, pure), `crates/daemon/src/disk.rs` (volumes, the
`disk` settings, the planner), `crates/daemon/src/server/disk.rs` (methods, triggers). CLI: `crates/cli/src/diskcmd.rs`.
App: `app/src/DiskPanel.tsx`, `strings.disk*.ts`, codes in `app/src/msg/disk.ts`.

## What is reclaimed

In a worktree's root (and its immediate child folders, for `package.json` projects such as `app/`):

- `target/` next to a `Cargo.toml`;
- `node_modules/` next to a `package.json`;
- the caches `.next`, `.turbo`, `.parcel-cache`, `.svelte-kit`, `.vite`, `dist`, `build`, `out`, only when git says they are
  ignored.

Every candidate must be ignored by git and hold no tracked file. A symlink or junction is skipped, and removal never follows a
reparse point out of the worktree. The next build makes them again. Never touched: the worktree folder, tracked files, commits,
branches, the project's own checkout.

Integrated worktrees are still removed whole by the janitor (see AGENTS.md, janitor bullet): a branch inside the default
branch, inside another live `ax/*` branch, merged by squash or cherry-pick (`git cherry` has no `+` line, or merging it would
change nothing), or with a merged PR. When such a worktree's folder is held, the janitor reports it once per episode as a
janitor event `kind: "pending"` with the holder (`janitor.pending {branch, path, holder, pid}`) or, when no holder is found,
the reason (`janitor.pending_busy {branch, path, detail}`), and retries by event.

## Inactive

A worktree's artifacts may go only when all of these hold (`artifacts::check`):

- it is not the project's main checkout (`artifacts.main`);
- no live or hibernated session, flow, orchestrator run, session waiting to be restored, or `axelstra lock` / `mem.hold`
  build uses it (`artifacts.busy`);
- it was quiet for `janitor.artifact_quiet_hours`: last use = the latest of a session leaving it and the mtimes of its git
  `HEAD`, `index` and `logs/HEAD` (`artifacts.recent {path, hours}`).

A user-forced reclaim (`force`) skips only the quiet rule, never the others. A failed removal is `artifacts.failed {path, detail}`.

## When (events, no timer)

- The janitor's wakes (daemon start, a session exits, `sweep_soon` after `done` or `git.merge`, a finished flow or run).
- ONE computed deadline: the moment the next worktree with artifacts becomes quiet; `next_janitor_wake` sleeps until it.
- `disk.ensure`: before a session spawn, before a worktree is created, and from `axelstra lock`. It reads the free space of the
  volumes holding `.axelstra-wt` and, when one is low, reclaims in a thread (a spawn or a build never waits for it).
- The memory governor's cause tick (it wakes anyway): a cheap free-space read.

Low space: free <= `min_free_gb` OR free <= `min_free_pct` % of the volume. The reclaim takes inactive worktrees oldest and
biggest first until free is above both lines plus `margin_gb`, or nothing is left. Still low: one `disk.low {volume, free_gb,
top}` notice per episode naming the biggest worktrees. Each reclaim is a janitor event `kind: "artifacts"` (toast) and
`disk.changed` on the watch stream. Sizes are measured on demand and cached (`memo.rs`) until a reclaim, a refresh or a
worktree added or removed; never scanned on a timer.

## Settings

In `settings.json` (`settings.get/set`):

- `janitor.artifact_quiet_hours`: 1..720, default 6.
- `disk`: `{enabled: true, min_free_gb: 30, min_free_pct: 10, margin_gb: 10}`. `enabled: false` stops both the janitor's
  artifact pass and the low-space reclaim; the user's own sweep still works.

## CLI and app

- `axelstra disk [--json] [--refresh]`: free space per volume, and per worktree its size, `target`, `node_modules`, caches,
  last use, whether it is integrated, and what can be reclaimed.
- `axelstra worktree sweep --artifacts [--dry-run] [--path <worktree>] [--force]`: reclaims now (`worktree.sweep {artifacts:
  true}`). `--force` and `--path` on another worktree are refused to agents (`agent_forbidden`).
- App: the Disk panel (from the status bar) lists volumes and worktree sizes with "Limpar builds" per worktree and for every
  inactive one; Settings has the `disk` and quiet-hours fields; a worktree with commits outside the default branch, inactive for
  over 24 h, gets a warning with Integrar / Limpar builds / Abrir.

## Research: incremental compilation in worktrees

Measured read-only on the existing target dirs of this repo's worktrees (Windows, no build run for it): `target/debug/incremental`
is 1.2-2.5 GB of 5.6-12 GB per worktree (15-35 %). Dependencies are never compiled incrementally; only the workspace crates are.
Turning it off (`CARGO_INCREMENTAL=0` or `[profile.dev] incremental = false`) would save that share, but every edit-and-rebuild
of `axelstra-daemon` while an agent iterates (`cargo check`, `cargo test`) would recompile whole crates. Not applied: the
artifact reclaim takes the whole `target/` of inactive worktrees, which is where the space is lost. `buildenv.rs` keeps
`CARGO_INCREMENTAL=0` only with `sccache` (which cannot cache incremental builds). `shared_target` stays off (see AGENTS.md,
Build cost).
