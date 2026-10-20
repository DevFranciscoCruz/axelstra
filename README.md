<p align="center"><img src="assets/logo.png" width="160" alt="Axelstra"></p>

# Axelstra

Lightweight desktop workbench for parallel coding agents on git worktrees (Windows, Tauri + Rust).
Projects, worktrees, terminals, file editor, changes/diff, a markdown kanban of ideas, and an
orchestrator that dispatches one agent per card and merges the results into an integration branch.

## Releases

`git tag vX.Y.Z && git push --tags` runs `.github/workflows/release.yml`: builds the NSIS installer
(with the daemon as sidecar) and attaches `Axelstra_<version>_x64-setup.exe` to the GitHub Release.
Installing and updating are manual: download it from the release and run it. The installer stops
the background daemon first, so running terminals and agents end.

Local build: `powershell -File scripts/build-installer.ps1` (prints the installer path).

The automatic updater (signed releases, `/latest` proxy) was built and then removed; it is in the
git history (commit `cbde50a`).

### macOS

Releases also carry a `.dmg` for Apple silicon. It is **not signed or notarized** (that needs a paid Apple developer account), so macOS blocks the first launch. Drag Axelstra to Applications, then either right-click the app and choose Open, or run:

```sh
xattr -dr com.apple.quarantine /Applications/Axelstra.app
```

Build locally with `sh scripts/prepare-sidecar.sh && cd app && npx tauri build --bundles dmg --config src-tauri/tauri.release.conf.json`.
