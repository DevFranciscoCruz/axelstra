# Scripts and tools

Helpers outside the Rust and TypeScript sources. Run everything from the repo root.

## Local gate

| Command | What it does |
| --- | --- |
| `node scripts/check.mjs` | `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test`, `tsc --noEmit`, `vitest`, and `actionlint` on `.github/workflows` (skipped with a notice when it is not on PATH, `--skip-workflows`); stops at the first failure |
| `node scripts/check.mjs --engine` | same, but the Rust steps cover only core, daemon and cli (no desktop toolkit needed) |
| `node scripts/check.mjs --skip-rust` / `--skip-frontend` | run one half |
| `--skip-fmt --skip-clippy --skip-test --skip-tsc --skip-vitest` | skip one step |
| `node scripts/check.mjs --fast` | quick loop, not the gate: clippy only on touched packages + dependents, no `cargo test`, frontend steps only when `app/` changed (vitest `--changed`, eslint on changed files) |
| `node scripts/check.mjs --self-test` | pure helpers only (`scripts/lib/jobs.mjs`, `scripts/lib/changed.mjs`), no compile |

In an Axelstra worktree wrap it: `axelstra lock verify -- node scripts/check.mjs`.

`rust-toolchain.toml` pins the Rust version (with clippy and rustfmt), `.nvmrc` pins Node. Bump the Rust pin on
purpose and fix the new lints in the same change.

## scripts/

- `stamp-version.mjs`: keeps the three version fields (Cargo workspace, `app/package.json`, `tauri.conf.json`) in step.
  `--check` (they agree), `--tag vX.Y.Z` (release workflow, `build-installer.ps1`), `--sync-lock` (the release PR:
  `Cargo.lock` follows the three fields), `--self-test` (CI).
- `next-version.mjs`: the next release version from the Conventional Commits since the last `v*` tag, same rules as
  `release-please-config.json`. `--json` (with counts per type), `--local [--above X.Y.Z]` (a local build: above the
  installed copy), `--self-test` (CI). See `docs/RELEASING.md`.
- `bump-version.mjs X.Y.Z`: sets all three fields with the same rules, then refreshes `Cargo.lock` offline
  (`cargo update --workspace --offline`). `--no-lock` skips the lock. Normally release-please's PR does this; see
  `docs/RELEASING.md`.
- `check-tauri-config.mjs`: fails when the CSP or the capabilities drift from what `SECURITY.md` says (no `style-src-attr 'unsafe-inline'`).
- `check-format.mjs`: prettier on the files this branch added or changed (or that are already formatted); `prettier --write` them.
- `check-i18n-static.mjs`: finds dictionary texts copied into module-level constants (part of `check.mjs`).
- `write-latest-json.mjs`: builds the updater manifest (`latest.json`) in the release workflow.
- `prepare-sidecar.sh` / `prepare-sidecar.ps1`: build the daemon binary and place it as the Tauri sidecar.
- `lib/jobs.mjs`: `jobsFor({cores, freeMemBytes, perJobBytes})`, the compile-job count for a machine; `check.mjs` sets `CARGO_BUILD_JOBS` from it when unset (never on CI). `node scripts/lib/jobs.mjs` prints today's value, `--self-test` tests it. `lib/changed.mjs`: which files and Rust packages a branch touched (for `--fast`).
- `build-installer.ps1`: **Windows only.** One command for the NSIS installer in steps (sidecar, frontend, bundle),
  printing time and peak memory per step. `-Version` overrides the version, `-Jobs` limits compile jobs (when omitted it is derived from the cores and the free memory, floor 2; an explicit value wins).
  It also sets `AXELSTRA_GIT_SHA` and `AXELSTRA_BUILD_DATE` (unless already set) so the sidecar and the app embed the same
  commit and date; the installer name does not change. Output: `target/release/bundle/nsis/Axelstra_<version>_x64-setup.exe`. Slow and heavy; do not run it for a code check.

## Build metadata

`axelstra_core::buildinfo::{VERSION, COMMIT, BUILT}` are consts filled by `crates/core/build.rs` from `AXELSTRA_GIT_SHA`
(short 7-char sha; fallback `git rev-parse --short=7 HEAD`, else `unknown`) and `AXELSTRA_BUILD_DATE` (UTC `YYYY-MM-DD`;
fallback today). Cargo reruns the script when either variable changes. Shallow checkouts have no usable history, so CI
sets the sha from `github.sha`. Shown by `axelstra --version` / `-V`, `axelstra version --json` and Settings > About.
Set them yourself only to reproduce a release build: `AXELSTRA_GIT_SHA=abc1234 AXELSTRA_BUILD_DATE=2026-01-31 cargo build`.

## Build environment of sessions and `lock --mem-mb`

Every session the daemon spawns gets `CARGO_BUILD_JOBS` (derived from cores and free memory when it starts), `RUSTC_WRAPPER`
and `CARGO_INCREMENTAL=0` only if `sccache` is on PATH, and a shared package-manager download cache for JS. It never overrides
a variable you already set. Edit `build` in `settings.json` (keys: `enabled`, `jobs`, `sccache` `auto`|`off`, `shared_target`,
`shared_js_cache`, `lock_budget_mb`, `lock_margin_mb`; details and the risks of `shared_target`, which is off, in AGENTS.md > Build cost).

- `axelstra lock verify -- <cmd>`: with no flag, declares a memory need from the jobs and is admitted while the holders fit the budget
  AND the memory available now minus that need leaves a 2 GB margin (`build.lock_margin_mb`). So several holders can run at once
  when memory allows; short memory waits for a holder to finish (no polling). If nobody holds the lock and memory is still short,
  it runs anyway after a warning. Use `--slots 1` for strict one-at-a-time.
- `axelstra lock NAME --mem-mb 3000 -- <cmd>`: declares the need yourself. `--slots N`: plain count slots (the old behaviour).
  The two flags exclude each other; keep one mode per name.

## tools/

- `measure-build.mjs [--label L] [--jobs 2,default] [--steps check,test-build,relink] [--json F]`: wall time and peak memory (process-tree sum, sampled every 500 ms, so a floor) of `cargo check --workspace`, `cargo test --no-run` for core/daemon/cli, and a relink, each from an empty `CARGO_TARGET_DIR` under the temp dir so runs are comparable. Run it inside the lock: `axelstra lock verify -- node tools/measure-build.mjs --label base --jobs 2`.

- `fake-agent.sh` / `fake-agent.ps1`: stand-in for a coding agent in tests. Behaviour is keyed on `AXELSTRA_TASK`:
  writes `<id>.txt` in the cwd and `.axelstra/result.json` (`{"status":"done",...}`); task `a` and `conflict` both write
  `shared.txt` (a merge conflict on purpose); `hang` sleeps 600 s; `flaky` exits 1 the first time (the marker file
  passed as `$1` / `-Marker` is how it remembers). Example: `AXELSTRA_TASK=a sh tools/fake-agent.sh /tmp/marker`.
- `smoke.mjs`: CLI-level smoke test. Starts `axelstra daemon` in a throwaway data dir (and, on Windows, a throwaway
  pipe name), checks `ls`, spawns and lists a session, kills it. `cargo build -p axelstra && node tools/smoke.mjs`.
  Runs in CI on Ubuntu and Windows. Never touches your real daemon.
- `idle-load.mjs [--bin p] [--worktrees 8] [--sessions 2] [--chatty 1] [--idle 45] [--max-spawns-per-min 2] [--json]`: idle-load check. Boots an
  isolated daemon (`AXELSTRA_DATA_DIR`, `AXELSTRA_TESTING=1`), makes a repo with N worktrees, connects like the app
  (`hello`, `watch.subscribe`), calls the polled methods once, then stays idle and reports from `debug.load` (calls per
  method, daemon-side; child processes counted by `core::proc::hidden`), the events the subscriber got per minute and new
  `lag.log` lines. Exits 1 on more than `--max-spawns-per-min` child processes per minute, any new `lag.log` line or any
  daemon call at idle. Run it before and after a change that touches polling, loaders or events.
- `idle-spawns.mjs [--bin p] [--projects 3] [--worktrees 10] [--agents 8] [--terminals 2] [--warmup 30] [--idle 180] [--agent-every 20] [--focus-every 120] [--commit-every 5] [--working 2|all] [--churn-every 90] [--no-remote] [--max-spawns-per-min N] [--json]`:
  spawn-storm check of a realistic desktop near idle. Same isolation as `idle-load.mjs` (plus a throwaway
  `GIT_CONFIG_GLOBAL` and an empty `GH_CONFIG_DIR`, so gh is logged out). Makes N repos (each with an unreachable
  github.com `origin`) and worktrees through `worktree.create` (auto, every fourth manual; some dirty, some with commits
  not in main), agent sessions (a shell holding its own agent credential: one main per project, the rest in worktrees),
  plain terminals and one finished command. Then acts like the app (a watch on the selected worktree and the loaders the
  frontend runs on each event, with its debounces; a window focus every `--focus-every` s) and like the agents (hook
  hints, `msg.inbox`, quiet `msg.send`, `session.list`; `--working` of them, 2 by default (`all` for a busy desktop), also
  edit a file and run `git status` in their worktree each turn and commit every `--commit-every` turns) and like dispatch
  (every `--churn-every` s a task worktree from `worktree.create` and a worker in it that ends: projects.changed,
  dispatch.changed, a janitor wake). After the warm-up it prints the child processes per minute by site and command (diff
  of `mem.status` breakdown.own.sites, which lists the 16 busiest; the rest is one row), calls per method with the
  children each call cost (a read that re-runs git in every worktree shows here) and events per minute. Exit 1 above
  `--max-spawns-per-min` (no limit when not given). The short in-process form is `crates/daemon/tests/idle_spawns.rs` (an
  edit may cost the git of the edited worktree only).
- `e2e.mjs [path\to\axelstra-app.exe]`: drives the real Windows app over the DevTools protocol (boot, a session, restore, the Add project dialog, CSP) in a throwaway data dir; see `docs/E2E.md`. Sets `AXELSTRA_TEST_HIDDEN=1` so no window shows.
- `e2e-html-link.mjs [path\to\axelstra-app.exe]`: Ctrl+click on an `.html` path in a terminal logs its `file://` url to `AXELSTRA_TEST_OPENER_LOG` instead of opening a browser; same isolation and hidden window as `e2e.mjs`, see `docs/E2E.md`.
- `e2e-kanban.mjs [path	oxelstra-app.exe]`: drag and drop on the Kanban board (pointer, Esc, drop outside, keyboard) with real input events, reading the `.kanban/` card files back; same isolation and hidden window as `e2e.mjs`, see `docs/E2E.md`.
- `winrect.mjs`: Win32 helpers for `e2e.mjs` and `visual.mjs` (window geometry, a real resize). Windows only.
- `visual.mjs [outDir]`: screenshots of the real app over CDP (Welcome, Settings, sidebar with lead rows and a stalled agent,
  three languages, 1280x800 and 800x600) in a throwaway data dir, with a `.cmd` stand-in agent. `VISUAL_FAST=1` skips the
  wait for the stall watchdog. `VISUAL_ADD=only` takes just the Add project dialog (the Pasta local, Clonar and Novo projeto tabs, 800x600, three languages; files `addproject-<local|clone|new>-<locale>.png`). Windows only; build the app and frontend first (see the header of the file).
- `make-demo-gif.mjs [--out F] [--width 820] [--fps 10] [--colors 255] [--tol 300] [--dump DIR --at ms,ms]`: regenerates `assets/demo/demo.gif` from the scripted mock `assets/demo/index.html` (fake data, pt-PT UI). Headless only: starts a headless Chrome, Edge or Chromium (set `AXELSTRA_BROWSER` to pick one) with no window, steps the mock's clock (`window.setT(ms)`) frame by frame over the DevTools protocol, and writes an optimized GIF itself (palette, transparent frame deltas, LZW). Needs node >= 22 and one such browser; no ffmpeg, gifski or npm packages. About 40 s to run; the result is about 600 KB. `--dump` also saves PNGs at the given video times so a few frames can be checked by eye. Open `assets/demo/index.html` in a browser to see the same animation live.
- `check-deny-expiry.mjs`: CI check that every advisory ignore in `deny.toml` still has a future `expires YYYY-MM-DD`.
- `make-icons.py`: regenerates the app icons from `assets/logo.png` (crop, black glow to alpha). Needs Python 3 and
  Pillow (`pip install pillow`). Only run it when the logo changes.

- `axelstra mem [--json]` / `axelstra resume <session>|--all`: memory pressure, the biggest sessions and what the governor paused; continue paused sessions. `axelstra lock` applies both memory gates: first it waits while the governor level is High or Critical (before taking a slot), then the memory-need gate above (budget and available memory). See [`MEMORY.md`](MEMORY.md).
- `axelstra disk [--json] [--refresh]` / `axelstra worktree sweep --artifacts [--dry-run] [--path P] [--force]`: free space of the volumes holding worktrees, size and build artifacts per worktree; reclaim the artifacts (`target/`, `node_modules/`, ignored caches) of inactive worktrees. See [`DISK.md`](DISK.md).

## Dependencies and policy

- `deny.toml`: cargo-deny policy (advisories, licenses, bans, sources). CI runs `cargo deny check`. Locally:
  `cargo install cargo-deny --locked --version 0.20.2 && cargo deny check`.
- `.github/dependabot.yml`: weekly updates for cargo, npm (`app/`) and GitHub Actions.
- `.github/workflows/dependency-audit.yml`: the `deny` and `audit` gates of `ci.yml` on a timer (Monday) and on PRs that
  change `Cargo.lock`, `app/package-lock.json` or `deny.toml`: `cargo deny check advisories licenses bans sources`,
  `tools/check-deny-expiry.mjs`, `npm audit --omit=dev --audit-level=high`. It exists for advisories published while
  nobody pushes and for ignores whose expiry passed; a failed scheduled run is the alarm (no issue is filed).
- `.github/workflows/codeql.yml`: CodeQL for `rust`, `javascript-typescript` and `actions` (all `build-mode: none`, no
  cargo build) on push and PR to `main` and weekly; results in Security > Code scanning. Code scanning's default setup
  must stay off (this is the advanced setup).
- Neither workflow is a required check, and must not become one: the release-please PR gets no `pull_request` runs
  (it is pushed with `GITHUB_TOKEN`; `release-please.yml` dispatches only `ci.yml` on it).

## Dev/test profile (measured)

`[profile.dev]` in the root `Cargo.toml` has `debug = "line-tables-only"` and `[profile.dev.package."*"]` has `debug = false`
(`cargo test` uses the dev profile). Release is untouched. Measured with `tools/measure-build.mjs --jobs 2`, one run each, from an
empty target dir, Windows MSVC, rustc 1.98.1:

| step | before | after |
|---|---|---|
| `cargo test --no-run` (core, daemon, cli): time | 174.1 s | 91.9 s |
| same: peak memory (process tree) | 2.06 GB | 1.18 GB |
| same: target dir | 8.31 GB | 3.78 GB |
| `cargo check --workspace`: time | 196.7 s | 225.9 s (noise: check emits no debuginfo; a shared machine) |
| same: peak memory / target dir | 2.42 GB / 1.53 GB | 2.46 GB / 1.52 GB |

Not applied: `split-debuginfo` (MSVC always writes a separate .pdb; the setting only matters on macOS/Linux, where the new
default is already good), and `rust-lld`/`lld-link` as linker (it would change the link of the statically linked sherpa-onnx
and is not safely optional across contributors' toolchains; not measured). Cost: a debugger shows no local variables in
dependencies and no types in our crates; backtraces and panics keep file:line. Use `CARGO_PROFILE_DEV_DEBUG=2` to get full debuginfo for a session.
