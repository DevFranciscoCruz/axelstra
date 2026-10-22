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
- **Voice:** dictate a prompt with `Ctrl+Shift+M`. Speech is recognized locally (Whisper, downloaded once, about 160 MB), stops by itself when you pause and goes straight to the active terminal.
- **Orchestration:** a coordinator agent splits work across worktrees and ephemeral workers that report back and disappear. Agents learn it from the `/axelstration` skill (installed for Claude Code and Codex); see below.
- **Search and usage:** `Ctrl+Shift+F` searches past Claude and Codex conversations and resumes one; tokens spent by the active Claude conversation are shown next to the tabs.
- **Status bar:** keep-awake switch, memory used by Axelstra and its terminals, number of terminals and listening dev-server ports.
- **Links:** URLs and file paths in terminal output are clickable (a path opens in the editor).
- **Notifications:** when an agent finishes, stops or needs an answer; clicking one opens that session.
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

The `.dmg` is the first install; after that the app updates itself like on Windows (the release carries a signed `.app.tar.gz` for the updater).

Releases are signed with a Developer ID and notarized when the repository has the Apple secrets (see *Releasing*). A build made without them is unsigned: macOS blocks the first launch, and you open it with right-click, Open, or

```sh
xattr -dr com.apple.quarantine /Applications/Axelstra.app
```

The first time you dictate, macOS asks for microphone access.

## Orchestration

Inside any Axelstra terminal the `axelstra` command is on the `PATH` and talks to the same daemon as the app, so whatever it starts shows up as a tab. Agents are taught to use it by the `/axelstration` skill, which the app installs in `~/.claude/skills` and `~/.agents/skills` (or run `axelstra install-skill`). The guide itself is served by the binary: `axelstra skills get axelstration`.

```sh
axelstra worktree create <project> <name>            # one worktree per task
axelstra agent <worktree> --prompt "..." --report-to  # ephemeral worker: reports back, then closes
axelstra run <worktree> --report-to -- cargo test     # one-shot command, no agent needed
axelstra ask <session> "question"                     # blocks until the answer (axelstra reply)
axelstra done "summary"                               # worker: report and finish
axelstra escalate "problem"                           # worker: ask the parent for help
axelstra flow plan.json                               # task graph with dependencies
axelstra limit 4                                      # at most 4 workers at once
```

The command palette entry *Orquestração* shows coordinators and their workers, with state and unread messages.

## Build from source

Needs Rust (stable), LLVM/libclang (for the speech engine; set `LIBCLANG_PATH` on Windows), Node 22 and, on Windows, the Tauri prerequisites (WebView2, MSVC build tools).

```sh
# Windows: daemon sidecar + frontend + NSIS installer; prints the installer path
powershell -File scripts/build-installer.ps1

# macOS
sh scripts/prepare-sidecar.sh && cd app && npx tauri build --bundles dmg --config src-tauri/tauri.release.conf.json
```

Local builds are not signed for the updater, so they never produce update artifacts. Engine tests: `cargo test -p axelstra-core -p axelstra-daemon -p axelstra`. Desktop host tests (`app/src-tauri`) need `cargo test --release -p axelstra-app` on Windows: the prebuilt static sherpa libs import `ucrtbased.dll`, so the debug test exe aborts at startup.

## Releasing

`git tag vX.Y.Z && git push --tags` runs `.github/workflows/release.yml`. It tests, builds the Windows installer and the macOS app and `.dmg` (daemon as sidecar, both signed for the updater), and attaches them to the GitHub Release. A last job merges the platform entries into `latest.json`, the manifest the app reads from `releases/latest/download/latest.json`. The version comes from the tag. The release notes are the commit subjects since the previous tag, so write them for the people who read the release; the same text shows in the update card.

The updater verifies every download against the public key in `app/src-tauri/tauri.conf.json`. The matching private key lives in the repo secrets `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`; losing it means existing installs can no longer update and must be reinstalled by hand.

The workflow can also be started manually with *dry run* on: it builds and signs, and keeps the installer on the run without publishing.

### Signing and notarizing the macOS app

Needs a paid Apple Developer account. Add these repository secrets and the next release is signed and notarized (without them it is still built, unsigned):

| Secret | What |
| --- | --- |
| `APPLE_CERTIFICATE` | Developer ID Application certificate exported as `.p12`, base64 (`base64 -i cert.p12`) |
| `APPLE_CERTIFICATE_PASSWORD` | the password chosen at export |
| `APPLE_SIGNING_IDENTITY` | e.g. `Developer ID Application: Your Name (TEAMID)` |
| `APPLE_ID` | the Apple ID email |
| `APPLE_PASSWORD` | an app-specific password for it (appleid.apple.com) |
| `APPLE_TEAM_ID` | the 10-character team id |

The hardened runtime entitlements (microphone, webview JIT) are in `app/src-tauri/entitlements.plist`.
