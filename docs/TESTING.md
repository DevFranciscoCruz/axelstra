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
- `AXELSTRA_TESTING=1` (or `cfg(test)`): `server::serve` / `serve_with_store` refuse to run when the projects store is in
  the real data dir or the pipe is the real default pipe (`proto::refuse_real_when_testing`). Proven by
  `crates/daemon/tests/isolation.rs` and the `proto` unit test.

## The helper

`crates/daemon/tests/common/mod.rs` (`mod common;` in the test file): the first call makes a temp data dir, sets both
variables for the whole test binary, and hands out `common::base_pipe()`, `common::pipe(tag)` (unique per call) and
`common::store(tag)` (a fresh `projects.json` in its own dir, for `serve_with_store`). Use `serve_with_store`, never
bare `serve`, in a test. Unit tests in `src` that need the data dir set `AXELSTRA_DATA_DIR` to a temp dir first (see
`isolate()` in `crates/cli/src/lock.rs`). Scripts (`tools/smoke.mjs`, `tools/e2e.mjs`) pass `AXELSTRA_DATA_DIR` plus a
temp `LOCALAPPDATA`.

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
