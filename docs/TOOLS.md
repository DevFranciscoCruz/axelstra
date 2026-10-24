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
- `check-deny-expiry.mjs`: CI check that every advisory ignore in `deny.toml` still has a future `expires YYYY-MM-DD`.
- `make-icons.py`: regenerates the app icons from `assets/logo.png` (crop, black glow to alpha). Needs Python 3 and
  Pillow (`pip install pillow`). Only run it when the logo changes.

## Dependencies and policy

- `deny.toml`: cargo-deny policy (advisories, licenses, bans, sources). CI runs `cargo deny check`. Locally:
  `cargo install cargo-deny --locked && cargo deny check`.
- `.github/dependabot.yml`: weekly updates for cargo, npm (`app/`) and GitHub Actions.
