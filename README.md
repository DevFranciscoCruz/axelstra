<p align="center"><img src="assets/logo.png" width="160" alt="Axelstra"></p>

# Axelstra

Lightweight desktop workbench for parallel coding agents on git worktrees (Windows, Tauri + Rust).
Projects, worktrees, terminals, file editor, changes/diff, a markdown kanban of ideas, and an
orchestrator that dispatches one agent per card and merges the results into an integration branch.

## Releases

`git tag vX.Y.Z && git push --tags` runs `.github/workflows/release.yml`: builds the NSIS installer
(with the daemon as sidecar), signs it with the Tauri updater key and attaches `setup.exe`,
`setup.exe.sig` and `latest.json` to the GitHub Release. The repo is private, so the app reads
updates through a server-side proxy (read-only token, short manifest cache) at the endpoint in
`plugins.updater.endpoints` of `app/src-tauri/tauri.conf.json`. The signing key lives outside the
repo (`~/.tauri/axelstra.key`); the repo only holds the public key. Losing it means existing
installs can never update.

Local build: `scripts/prepare-sidecar.ps1`, then in `app/` with `TAURI_SIGNING_PRIVATE_KEY` and
`TAURI_SIGNING_PRIVATE_KEY_PASSWORD` set: `npx tauri build --config src-tauri/tauri.release.conf.json`.
