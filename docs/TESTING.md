# Testing: never touch the real data dir

**Rule: tests never use the user's real data dir (`%LOCALAPPDATA%\axelstra` on Windows) nor the real pipe.** A test
that starts a daemon (or runs `axelstra`, which reads the token, locks and flows from the data dir) uses the helper
below. Why: a test daemon once wrote its own test binary into the real `hooks.json` (`statusLine`), and every new Claude
session lost its status line (ROADMAP B2/T4).

## The switch

- `AXELSTRA_DATA_DIR=<path>` replaces the data dir for every process that reads `proto::data_dir()` (token,
  `projects.json`, `sessions.json`, `hooks.json`, `settings.json`, `notes/`, `prompts/`, `scrollback/`, `status/`, `sid/`,
  `flows/`, `locks/`, `dispatch.json`, `lag.log`, `statusline.log`). On Windows the pipe name derives from it
  (`\.\pipe\axelstra-<SID>-d<hash of the dir>`), on Unix the socket lives inside it. Unset = the real, SID-based default.
  `AXELSTRA_TEST_PIPE` still appends a suffix on top.
- `AXELSTRA_TESTING=1` (or `cfg(test)`; also what lets `project.clone` accept `file://` so tests can clone a local bare repo): `server::serve` / `serve_with_store` refuse to run when the projects store is in
  the real data dir or the pipe is the real default pipe (`proto::refuse_real_when_testing`). Proven by
  `crates/daemon/tests/isolation.rs` and the `proto` unit test.

## The helper

`crates/daemon/tests/common/mod.rs` (`mod common;` in the test file): the first call makes a temp data dir, sets both
variables for the whole test binary, and hands out `common::base_pipe()`, `common::pipe(tag)` (unique per call) and
`common::store(tag)` (a fresh `projects.json` in its own dir, for `serve_with_store`). Use `serve_with_store`, never
bare `serve`, in a test. Unit tests in `src` that need the data dir set `AXELSTRA_DATA_DIR` to a temp dir first (see
`isolate()` in `crates/cli/src/lock.rs`). Scripts (`tools/smoke.mjs`, `tools/e2e.mjs`) pass `AXELSTRA_DATA_DIR` plus a
temp `LOCALAPPDATA`.

### Lock files (`locks/`)

`axelstra lock NAME` keeps one file per slot, `locks/NAME-<slot>.lock` (slot 0..`--slots`-1), as an OS file lock. There is no
per-process file, ticket or queue file: the OS releases the lock when the owner dies, and a named file is shared by every
caller, so it is never deleted (deleting a file another process holds would break mutual exclusion). The `s<pid>-<n>.lock`
and `t<pid>-<n>.lock` files seen in the real data dir were made by the unit tests in `lock.rs` (lock names `s<pid>`/`t<pid>`)
before they were isolated (commit a3009b5, 2026-10-24). Since then the tests use a temp `AXELSTRA_DATA_DIR` and remove their
own files (`Cleanup` in `lock.rs`). The old ones can be deleted by hand (`locks/[st]<digits>-<digits>.lock`).

Memory-budget locks (`--mem-mb`) use `locks/NAME-mem-<unit>.lock` (unit = 512 MB of the budget); same rules: OS file locks, never
deleted, tests use a temp data dir and `CleanupMem`. The admission step is pure (`buildenv::admit`, tested in `buildenv.rs`), and
`buildenv` tests take env, PATH, free memory and cores as parameters, so none of them compiles or reads the real machine.

## Audit (every test or script that can reach the data dir or the pipe)

| Where | What it reaches | Isolation |
|---|---|---|
| `crates/daemon/tests/{conn,roundtrip,orch_pty}.rs` | `serve()` on the real dir (was the leak) | `serve_with_store` + `common::store` |
| `crates/daemon/tests/*.rs` (all others) | pipe base from `pipe_name()`, `lag.log` and token via `data_dir()` | `common::base_pipe()` sets the temp dir |
| `crates/daemon/tests/isolation.rs` | the guards themselves | refuses; only reads paths |
| `crates/daemon/src/**` `#[cfg(test)]` | no `serve`, `data_dir`, token or pipe use | none needed |
| `crates/cli/src/lock.rs` tests | `ensure_data_dir()/locks` | `isolate()` temp `AXELSTRA_DATA_DIR` |
| `crates/cli/tests/lock_unix.rs` | spawns `axelstra lock` | `HOME`, `XDG_DATA_HOME`, `AXELSTRA_DATA_DIR` temp |
| `crates/cli/src` other tests (statusline, orchcmd, hook, ctl, dispatch) | pure functions, temp dirs of their own | none needed |
| `app/src-tauri` tests | no data dir, pipe or daemon | none needed |
| `tools/smoke.mjs`, `tools/e2e.mjs` | spawn daemon, CLI and app | temp `LOCALAPPDATA` + `AXELSTRA_DATA_DIR` + `AXELSTRA_TEST_PIPE` |
| `scripts/*.mjs` | none start a daemon | n/a |

## Proof that a run left the real dir alone

`node scripts/check.mjs` snapshots (SHA-256) `hooks.json`, `token`, `projects.json` and `settings.json` of the real data
dir before the `test` step and fails if any changed after it; it is skipped when that dir does not exist. Limits: only
those four files are compared, because a real running daemon legitimately rewrites `sessions.json` (and backups),
`lag.log`, `statusline.log` and `status/` on its own; a leak into those would not be caught by the script. It also
cannot see a test that only reads. The `serve` refusal is the real barrier; the script is the net.

Tests of Add project (`crates/daemon/tests/sources.rs`, the `addproject` scenario of `tools/e2e.mjs`) create every repository under a temp dir, give git an identity
through a throwaway `GIT_CONFIG_GLOBAL` (never the developer's gitconfig), and keep `gh` logged out with an empty `GH_CONFIG_DIR`. `project.discover` scans
`USERPROFILE`/`HOME`, i.e. the real home: both the daemon test and the app E2E (which cannot override `USERPROFILE`, see `docs/E2E.md`) only check the shape of the list and never add from it.

App-level drivers (`tools/e2e.mjs`, `tools/visual.mjs`) set `AXELSTRA_TEST_HIDDEN=1` so the app window is created transparent
and unfocusable (see `docs/E2E.md`). Never launch the real app from a test without it.

## Build metadata in tests

Tests that print or compare the version must not hard-code the commit or date: they change on every build. Compare
against `axelstra_core::buildinfo::{VERSION, COMMIT, BUILT}`, or set `AXELSTRA_GIT_SHA`/`AXELSTRA_BUILD_DATE` for the build
under test (see `docs/TOOLS.md`).

## Memory governor in tests

The governor's tests drive the levels with the fake `MemSource` (`governor::fake::FakeSource`) and freeze nothing real: pause tests inject `FakeSuspender`. Never suspend a process the test does not own: the only real freeze (`governor/suspend.rs`) is of a child the test starts and kills itself, and it never targets the daemon or pids 0/1. A test must not push the machine into real memory pressure.

## Opening HTML from a terminal

`files.resolve_html` (daemon) turns a path clicked in a terminal into a `file://` URL; the host command `open_html_file`
({session, path}) calls it itself and hands the returned URL to the system opener, so the page never passes a URL.
Rules (`crates/daemon/src/htmlpath.rs`): trailing `. , ; : ) ]` and `:line[:col]` are dropped; the real path (symlinks
followed) must be a regular `.html`/`.htm` file. A relative path must stay inside the session's cwd, worktree or project
root (so `..` and symlinks cannot leave, and `/a/proj-evil` is not inside `/a/proj`). An absolute path is allowed anywhere:
the user clicked it explicitly and only html/htm pass, never an executable or script. Agents may not call it
(`agent_forbidden`). In tests and E2E set `AXELSTRA_TEST_OPENER_LOG=<file>`: the host appends each URL to that file
instead of starting a browser.

## Revealing, opening and running a clicked path

Ctrl+click on a path that is not code or html (`app/src/terminal/pathOpen.ts`) asks the daemon (`files.resolve_path`) what it is and
shows a dialog; the buttons call `files.path_action {session, path, action: reveal|open|run}`. Rules (`crates/daemon/src/pathopen.rs`):
the path must exist and is canonical (symlinks followed); `run` needs a regular program or script (extension list or exec bit) and
`open` a regular non-program file, both only when the real path is inside the session's cwd, worktree or project; `reveal`
works anywhere, also for folders. The daemon re-resolves on every call and refuses what the description did not allow, nothing goes
through a shell (`ShellExecuteW` with the path as one argument; `open`/`xdg-open`, or the file itself with its exec bit, on Unix), both
methods are in `agent_forbidden`, and a run is written to `audit.log` (`files.run_path`). A remote (WSL/SSH) worktree's dialog only
offers to copy the path. With `AXELSTRA_TEST_OPENER_LOG=<file>` the daemon appends `<action>\t<path>` to that file instead of doing
anything (`crates/daemon/tests/path_open.rs`); tests must set it, never reveal or run anything real.

## Cheaper builds and runs

Machines with little memory should not compile in parallel at full width. Rules that apply to every heavy command:

- Inside an Axelstra worktree run it as `axelstra lock verify -- <cmd>` (one at a time across sessions).
- `CARGO_BUILD_JOBS`: `node scripts/check.mjs` derives it when it is not set and `CI` is not (`scripts/lib/jobs.mjs`:
  `min(cores / 2, (free memory - 2 GB) / 1.5 GB)`, at least 1); an explicit value always wins. CI is never touched.
- `node scripts/check.mjs --fast` is the loop while working: fmt, clippy (which type-checks the tests, nothing is linked) only on
  the packages the branch touched and their dependents, tsc/eslint/vitest only when `app/` changed, no `cargo test`.
  It is not the gate: CI and the push gate run the full mode, which is unchanged.
- `node scripts/check.mjs --self-test` checks the pure helpers (jobs for a machine, affected packages) in milliseconds, no compile.
- Numbers behind the settings (wall time, peak memory, per jobs value) come from `tools/measure-build.mjs`; see `docs/TOOLS.md`.
