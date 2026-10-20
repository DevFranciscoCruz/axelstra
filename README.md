<p align="center"><img src="assets/logo.png" width="160" alt="Axelstra"></p>

# Axelstra

Lightweight desktop workbench for parallel coding agents on git worktrees (Windows and macOS, Tauri + Rust).

- **Projects and worktrees:** one sidebar entry per repo, one worktree per task, each with its own terminals.
- **Agents:** open Claude Code or Codex in a worktree with one click; a dot shows working / idle / done and you get a notification when an agent stops.
- **Terminals:** split view, command palette (`Ctrl+K`), dev-server ports shown as chips that open in the browser.
- **Session restore:** terminals live in a background daemon. After a reboot or an update they reopen where they were, and Claude Code resumes its conversation (a `/clear` is followed, so you get the new one).
- **Code:** file tree and editor, changes panel with diff and line comments sent to a running agent, commit box, pull requests and issues (needs `gh`).
- **Kanban and orchestrator:** a markdown board of ideas; the orchestrator dispatches one agent per card and merges the results into an integration branch.
- **Automations:** scheduled agent runs in fresh worktrees (interval, daily, weekly).
- **Layout:** `Ctrl+B` hides the project list, `Ctrl+Shift+B` the right panel (or use the panel buttons in the tab bar).

## Install

Download the latest installer from the [Releases page](https://github.com/DevFranciscoCruz/axelstra/releases/latest):

| Platform | File |
| --- | --- |
| Windows (x64) | `Axelstra_<version>_x64-setup.exe` |
| macOS (Apple silicon) | `Axelstra_<version>_aarch64.dmg` |

Requirements: `git`, plus the agent CLI you want to use (`claude` or `codex`) on your `PATH`; `gh` for the pull requests panel.

### Updates (Windows)

Axelstra checks GitHub Releases at launch, every 6 hours and when the window regains focus. A newer version is downloaded and verified in the background; when it is ready a card appears in the corner: **Reiniciar agora** installs it and reopens the app, **Mais tarde** hides the card. A downloaded update is also installed when you close the app.

Terminals and agents are not interrupted: they belong to the background daemon, and the installer moves the running daemon aside instead of stopping it. The new window reconnects to it. The daemon only picks up new code the next time it starts (reboot, or when the app finds it too old to talk to, in which case its terminals are restored).

The first install is manual. Versions older than the first release with the updater do not update themselves: install the new one once over the old one.

### macOS

The `.dmg` is **not signed or notarized** (that needs a paid Apple developer account), so macOS blocks the first launch, and there is no in-app updater on macOS: download each new `.dmg` yourself. Drag Axelstra to Applications, then either right-click the app and choose Open, or run:

```sh
xattr -dr com.apple.quarantine /Applications/Axelstra.app
```

## Build from source

Needs Rust (stable), Node 22 and, on Windows, the Tauri prerequisites (WebView2, MSVC build tools).

```sh
# Windows: daemon sidecar + frontend + NSIS installer; prints the installer path
powershell -File scripts/build-installer.ps1

# macOS
sh scripts/prepare-sidecar.sh && cd app && npx tauri build --bundles dmg --config src-tauri/tauri.release.conf.json
```

Local builds are not signed for the updater, so they never produce update artifacts. Engine tests: `cargo test -p axelstra-core -p axelstra-daemon -p axelstra`.

## Releasing

`git tag vX.Y.Z && git push --tags` runs `.github/workflows/release.yml`. It tests, builds the Windows installer (signed for the updater, daemon as sidecar) and the macOS `.dmg`, and attaches them to the GitHub Release together with `latest.json`, the manifest the app reads from `releases/latest/download/latest.json`. The version comes from the tag.

The updater verifies every download against the public key in `app/src-tauri/tauri.conf.json`. The matching private key lives in the repo secrets `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`; losing it means existing installs can no longer update and must be reinstalled by hand.

The workflow can also be started manually with *dry run* on: it builds and signs, and keeps the installer on the run without publishing.
