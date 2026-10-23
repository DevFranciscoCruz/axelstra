<p align="center"><img src="assets/logo.png" width="160" alt="Axelstra"></p>

# Axelstra

Lightweight desktop workbench for parallel coding agents on git worktrees (Windows and macOS, Tauri + Rust).

<p align="center"><img src="assets/demo/demo.gif" width="820" alt="Axelstra tour: new worktree with an agent, notification, Ctrl+K, AI commit message, files and editor, Kanban, automations and dispatcher"></p>

<sub>Scripted mock with fake data: source in [`assets/demo/index.html`](assets/demo/index.html), open it in a browser to watch it live. The tour: create a worktree and start an agent, get notified when another one finishes, jump with `Ctrl+K`, write the commit message with AI, browse and edit files, run the Kanban, scheduled automations and the dispatcher.</sub>

- **Projects and worktrees:** one sidebar entry per repo, one worktree per task, each with its own terminals.
- **Agents:** open Claude Code or Codex in a worktree with one click; a dot shows working / idle / done and you get a notification when an agent stops.
- **Terminals:** split view, command palette (`Ctrl+K`), dev-server ports shown as chips that open in the browser.
- **Session restore:** terminals live in a background daemon. After a reboot or an update they reopen where they were, and Claude Code resumes its conversation (a `/clear` is followed, so you get the new one).
- **Code:** file tree and editor, changes panel with diff and line comments sent to a running agent, commit box with an AI-suggested message, pull requests and issues (needs `gh`).
- **Ship flow with CI:** once the worktree's branch has a pull request, the Changes panel shows its CI state (passing, failing, running, none) with every check and its link, the review decision and mergeability, refreshed every 20 seconds while the panel is visible and the window focused. Buttons open it in the browser, enable auto-merge or squash-merge it (always behind a confirmation, with an option to delete the branch). Needs `gh` logged in.
- **Notification center:** a bell with an unread badge lists the last 100 events, newest first: agent finished or waiting, CI turned failing or passing, merge done, errors. Click an entry to jump to its worktree. Kept across restarts; `Esc` closes it.
- **Kanban and orchestrator:** a markdown board of ideas; the orchestrator dispatches one agent per card and merges the results into an integration branch.
- **AI commit messages:** the sparkle button in the commit box asks your `claude` CLI for a message from the pending diff. It follows the repo's recent style (e.g. Conventional Commits), writes an imperative subject of at most 72 characters and a body only when the reason is not obvious. You read and edit it before committing; nothing is staged until you commit.
- **Worktree cleanup:** a background sweep removes `ax/*` worktrees once their work is merged (in `origin/main`, or its pull request merged on GitHub). It never touches a worktree with a live session, uncommitted changes or unmerged work. On by default; `axelstra worktree sweep --auto off` turns it off, `--dry-run` lists what it would remove.
- **Automations:** scheduled agent runs in fresh worktrees (interval, daily, weekly).
- **Voice:** dictate a prompt with `Ctrl+Shift+M`. Speech is recognized locally (Whisper, downloaded once, about 160 MB), stops by itself when you pause and goes straight to the active terminal.
- **Orchestration:** a coordinator agent splits work across worktrees and ephemeral workers that report back and disappear. Agents learn it from the `/axelstration` skill (installed for Claude Code and Codex); see below.
- **Search and usage:** `Ctrl+Shift+F` searches past Claude and Codex conversations and resumes one; tokens spent by the active Claude conversation are shown next to the tabs, and a `Σ` chip lists the tokens (input, output, cache) each worktree has spent across its Claude and Codex conversations, read from their logs. Tokens only: no cost is estimated.
- **Status bar:** keep-awake switch, memory used by Axelstra and its terminals, number of terminals, listening dev-server ports and the total agent tokens (hover for the per-worktree breakdown).
- **Links:** URLs and file paths in terminal output are clickable (a path opens in the editor).
- **Notifications:** when an agent finishes, stops or needs an answer. On Windows, clicking the toast opens that session. On macOS and Linux the notification is shown but clicking it does not select the session (the notification plugin has no click callback there); the worktree gets its attention dot instead.
- **Layout:** `Ctrl+B` hides the project list, `Ctrl+Shift+B` the right panel (or use the panel buttons in the tab bar).

## Install

Download the latest installer from the [Releases page](https://github.com/DevFranciscoCruz/axelstra/releases/latest):

| Platform | File |
| --- | --- |
| Windows (x64) | `Axelstra_<version>_x64-setup.exe` |
| macOS (Apple silicon) | `Axelstra_<version>_aarch64.dmg` |

Requirements: `git`, plus the agent CLI you want to use (`claude` or `codex`) on your `PATH`; `gh` for the pull requests panel.

### Updates (Windows)

Axelstra checks GitHub Releases at launch, every 6 hours and when the window regains focus. A newer version is downloaded and verified in the background; when it is ready a card appears in the corner: **Reiniciar agora** installs it and reopens the app, **Mais tarde** hides the card. A downloaded update is also installed when you close the app: quitting goes through the exit hook in `app/src-tauri/src/main.rs`, which installs the pending update without relaunching.

Terminals and agents are not interrupted: they belong to the background daemon, and the installer moves the running daemon aside instead of stopping it. The new window reconnects to it. The daemon only picks up new code the next time it starts (reboot, or when the app finds it too old to talk to, in which case its terminals are restored).

The first install is manual. Versions older than the first release with the updater do not update themselves: install the new one once over the old one.

### Linux (experimental)

Linux is not a supported platform yet. The release workflow builds a `.deb` and an AppImage in an experimental job that never blocks a release, and CI checks that the desktop host compiles on Ubuntu. Nobody has run the result on a Linux desktop, so expect rough edges. Known state:

- The files are not signed for the updater: a Linux install does not update itself, and `latest.json` has no Linux entry. Install each new release by hand.
- Keep-awake uses `systemd-inhibit` when it is on `PATH`. Without it the switch shows `n/d` and does nothing.
- Notification clicks do not open the session (see *Notifications*).

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

Needs Rust (stable), LLVM/libclang (for the speech engine; set `LIBCLANG_PATH` on Windows), Node 22 and, on Windows, the Tauri prerequisites (WebView2, MSVC build tools). On Linux (experimental) install the Tauri 2 system packages first: `libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev libxdo-dev libssl-dev patchelf`.

```sh
# Windows: daemon sidecar + frontend + NSIS installer; prints the installer path
powershell -File scripts/build-installer.ps1

# macOS
sh scripts/prepare-sidecar.sh && cd app && npx tauri build --bundles dmg --config src-tauri/tauri.release.conf.json

# Linux (experimental)
sh scripts/prepare-sidecar.sh && cd app && npx tauri build --config src-tauri/tauri.release.linux.conf.json
```

Local builds are not signed for the updater, so they never produce update artifacts. Engine tests: `cargo test -p axelstra-core -p axelstra-daemon -p axelstra`. Desktop host tests (`app/src-tauri`): `cargo test -p axelstra-app`. On Windows the prebuilt static sherpa libs import the debug CRT (`ucrtbased.dll`), so `app/src-tauri/build.rs` drops the debug CRT libs from debug builds and `src/debugcrt.rs` stands in for the two debug heap functions that leaves unresolved; if that ever breaks, `cargo test --release -p axelstra-app` still works.

## Releasing

`git tag vX.Y.Z && git push --tags` runs `.github/workflows/release.yml`. It runs the engine tests, the type check and the frontend tests, builds the Windows installer and the macOS app and `.dmg` (daemon as sidecar, both signed for the updater), and attaches them to the GitHub Release. An experimental Linux job (`.deb` and AppImage, unsigned for the updater, allowed to fail) adds its files afterwards. A last job merges the platform entries into `latest.json`, the manifest the app reads from `releases/latest/download/latest.json`. The version comes from the tag. The release notes are the commit subjects since the previous tag, so write them for the people who read the release; the same text shows in the update card.

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
