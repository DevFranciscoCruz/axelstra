# Scripts and tools

Helpers outside the Rust and TypeScript sources. Run everything from the repo root.

## Local gate

| Command | What it does |
| --- | --- |
| `node scripts/check.mjs` | `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test`, `tsc --noEmit`, `vitest`; stops at the first failure |
| `node scripts/check.mjs --engine` | same, but the Rust steps cover only core, daemon and cli (no desktop toolkit needed) |
| `node scripts/check.mjs --skip-rust` / `--skip-frontend` | run one half |
| `--skip-fmt --skip-clippy --skip-test --skip-tsc --skip-vitest` | skip one step |

In an Axelstra worktree wrap it: `axelstra lock verify -- node scripts/check.mjs`.

`rust-toolchain.toml` pins the Rust version (with clippy and rustfmt), `.nvmrc` pins Node. Bump the Rust pin on
purpose and fix the new lints in the same change.

## scripts/

- `stamp-version.mjs`: keeps the three version fields (Cargo workspace, `app/package.json`, `tauri.conf.json`) in step.
  `--check` (they agree), `--tag vX.Y.Z` (release workflow), `--self-test` (CI).
- `bump-version.mjs X.Y.Z`: sets all three fields with the same rules, then refreshes `Cargo.lock` offline
  (`cargo update --workspace --offline`). `--no-lock` skips the lock. Releases still go by tag; see AGENTS.md.
- `check-tauri-config.mjs`: fails when the CSP or the capabilities drift from what `SECURITY.md` says (no `style-src-attr 'unsafe-inline'`).
- `check-format.mjs`: prettier on the files this branch added or changed (or that are already formatted); `prettier --write` them.
- `check-i18n-static.mjs`: finds dictionary texts copied into module-level constants (part of `check.mjs`).
- `write-latest-json.mjs`: builds the updater manifest (`latest.json`) in the release workflow.
- `prepare-sidecar.sh` / `prepare-sidecar.ps1`: build the daemon binary and place it as the Tauri sidecar.
- `build-installer.ps1`: **Windows only.** One command for the NSIS installer in steps (sidecar, frontend, bundle),
  printing time and peak memory per step. `-Version` overrides the version, `-Jobs` limits compile jobs.
  Output: `target/release/bundle/nsis/Axelstra_<version>_x64-setup.exe`. Slow and heavy; do not run it for a code check.

## tools/

- `fake-agent.sh` / `fake-agent.ps1`: stand-in for a coding agent in tests. Behaviour is keyed on `AXELSTRA_TASK`:
  writes `<id>.txt` in the cwd and `.axelstra/result.json` (`{"status":"done",...}`); task `a` and `conflict` both write
  `shared.txt` (a merge conflict on purpose); `hang` sleeps 600 s; `flaky` exits 1 the first time (the marker file
  passed as `$1` / `-Marker` is how it remembers). Example: `AXELSTRA_TASK=a sh tools/fake-agent.sh /tmp/marker`.
- `smoke.mjs`: CLI-level smoke test. Starts `axelstra daemon` in a throwaway data dir (and, on Windows, a throwaway
  pipe name), checks `ls`, spawns and lists a session, kills it. `cargo build -p axelstra && node tools/smoke.mjs`.
  Runs in CI on Ubuntu and Windows. Never touches your real daemon.
- `e2e.mjs [path\to\axelstra-app.exe]`: drives the real Windows app over the DevTools protocol (boot, a session, restore, CSP) in a throwaway data dir; see `docs/E2E.md`. Sets `AXELSTRA_TEST_HIDDEN=1` so no window shows.
- `e2e-html-link.mjs [path\to\axelstra-app.exe]`: Ctrl+click on an `.html` path in a terminal logs its `file://` url to `AXELSTRA_TEST_OPENER_LOG` instead of opening a browser; same isolation and hidden window as `e2e.mjs`, see `docs/E2E.md`.
- `winrect.mjs`: Win32 helpers for `e2e.mjs` and `visual.mjs` (window geometry, a real resize). Windows only.
- `visual.mjs [outDir]`: screenshots of the real app over CDP (Welcome, Settings, sidebar with lead rows and a stalled agent,
  three languages, 1280x800 and 800x600) in a throwaway data dir, with a `.cmd` stand-in agent. `VISUAL_FAST=1` skips the
  wait for the stall watchdog. `VISUAL_ADD=only` takes just the Add project dialog (choice and remote form, 800x600, three languages). Windows only; build the app and frontend first (see the header of the file).
- `check-deny-expiry.mjs`: CI check that every advisory ignore in `deny.toml` still has a future `expires YYYY-MM-DD`.
- `make-icons.py`: regenerates the app icons from `assets/logo.png` (crop, black glow to alpha). Needs Python 3 and
  Pillow (`pip install pillow`). Only run it when the logo changes.

## Dependencies and policy

- `deny.toml`: cargo-deny policy (advisories, licenses, bans, sources). CI runs `cargo deny check`. Locally:
  `cargo install cargo-deny --locked && cargo deny check`.
- `.github/dependabot.yml`: weekly updates for cargo, npm (`app/`) and GitHub Actions.
