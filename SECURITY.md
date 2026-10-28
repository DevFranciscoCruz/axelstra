# Security policy

## Reporting a vulnerability

Do not open a public issue. Report it privately through GitHub: the repository's **Security** tab,
**Report a vulnerability** (private security advisory). Include the version (`axelstra --version`),
your OS, and steps to reproduce.

Expect an acknowledgement as soon as the maintainer sees it. Fixes ship in the next release.

## Scope

- The desktop app, the daemon and the `axelstra` CLI.
- The daemon listens only on a local socket and authenticates clients with a shared secret stored in
  the per-user data directory (`token`). Anything that lets another local user or a remote party
  bypass that is in scope.
- The in-app updater only installs archives signed with the project's updater key.

Supported versions: the latest release only.

## Content Security Policy

The production policy lives in `app/src-tauri/tauri.conf.json` (`app.security.csp`) and is guarded by
`scripts/check-tauri-config.mjs`.

- **Scripts:** `script-src 'self'`. No `unsafe-inline`, no `unsafe-eval`. The one inline script in `app/index.html`
  is hashed into `script-src` by Tauri at build time.
- **Style elements: no `unsafe-inline`.** `style-src 'self'` plus a per-load nonce. xterm's DOM renderer (used by every
  terminal that is not the visible one) and CodeMirror create `<style>` elements at runtime, and xterm has no nonce
  option. `app/index.html` carries one `<style>` so Tauri injects a random nonce into that tag and into `style-src`
  on every page load, and an inline script wraps `document.createElement` so each `<style>` element the page creates
  gets that nonce. A `<style>` injected as markup (no nonce) is refused; `tools/e2e.mjs` checks that on the real app.
- **Style attributes: no `unsafe-inline` either** (no `style-src-attr`, so it falls back to `style-src 'self'`). Solid compiles a
  fully static `style={{...}}` into a `style="..."` attribute inside its template HTML, which CSP blocks. So `app/src` has no
  static style objects: they are CSS classes (`styles.css`, Tailwind utilities). Styles with a runtime value (`style={{ width: x }}`,
  `style={FLOW}`, `style={{ "--left-w": ... }}`) compile to `el.style.setProperty` calls, which are CSSOM and not governed by
  `style-src-attr`; those stay. Rule: a `style={{}}` must hold at least one runtime value, and a literal-only property in it is
  moved to a class (a mixed object puts its literal parts in the template, so split them out too). Third-party templates with a
  literal `style=` (Kobalte's `HiddenSelect` and popper arrow) are not rendered by this app. `tools/e2e.mjs` fails on a CSP violation.
- **Images:** `img-src 'self' data: https://github.com https://avatars.githubusercontent.com`. The project icon is
  `https://github.com/<owner>.png`, which redirects to `avatars.githubusercontent.com`; both hosts are needed.
  `blob:` was dropped: nothing in the app creates blob images.
- **Also:** `connect-src` is the Tauri IPC only, `object-src`, `base-uri`, `frame-src`, `form-action` and
  `frame-ancestors` are `'none'`. `devCsp` is looser (inline scripts and styles, the Vite dev origin) and applies
  only to `tauri dev` builds.

## Supply chain notes

- **Sherpa speech libraries.** `sherpa-rs` (feature `download-binaries`) downloads the prebuilt static sherpa-onnx
  archive at build time. Its build script (`sherpa-rs-sys/build.rs`) compares the SHA-256 of the download with the
  value in `dist.json` / `checksum.txt` that ships *inside the crate* and panics on a mismatch. The crate itself is
  pinned by `Cargo.lock`, so the archive is pinned transitively; a separate checksum here would duplicate that and
  could only drift. Never set `UNSAFE_DISABLE_CHECKSUM_VALIDATION` in CI or release builds. Bumping `sherpa-rs`
  bumps the archive: review its `dist.json` tag in the same change.
- **Workflows.** Third-party actions are pinned by full commit SHA (with the version in a comment), `actionlint` is
  clean, and `cargo deny check advisories licenses bans sources` runs in CI.


## Paths clicked in a terminal

Terminal output is untrusted text (an agent or a program prints it). Ctrl+click on a path never runs, opens or reveals
anything by itself: code and text inside the worktree open in the editor, `.html` in the browser, everything else
(programs, scripts, folders, binaries, paths outside the project) shows a dialog with the canonical path and its type.
Focus starts on Cancel (Enter and Esc cancel) and the buttons only react to a real user click (`isTrusted`). The action
is `files.path_action`: the daemon resolves the path again (exists, canonical, symlinks followed), only runs a regular
program or script whose real path is inside the session's cwd, worktree or project, never passes through a shell
(`ShellExecuteW` with the path as one argument, `open`/`xdg-open`, or the file itself on Unix), is closed to agents
(`agent_forbidden`) and writes every run to the audit log. A path outside the project can only be shown in its folder;
a remote (WSL/SSH) worktree only offers to copy the path. Anyone who clicks Run runs that program with their own permissions.

## Antivirus exclusions of the worktree folders

Excluding a folder from Windows Defender trades safety for speed: files in it are no longer scanned in real time, so
malware written there is found later (or by a manual scan). Therefore:

- It is **opt-in**. Nothing is excluded by default; the user ticks a consent box and clicks Apply in Settings > Desempenho,
  after reading the trade-off, and Windows asks for administrator permission (UAC) before anything changes.
- **Scope:** only the `.axelstra-wt` folders of registered projects. The daemon refuses any other path (`scanx.bad_path`),
  never a drive, a process or a file type. The projects' own checkouts are never excluded.
- **Never agent-triggerable:** every `scanx.*` method is in `agent_forbidden`; an agent cannot apply, revert or probe it.
  (The limits in "Limits of the agent role" still apply to a hostile process outside the session tree.) Each apply and revert is
  written to the audit log.
- **Reversible:** Revert removes exactly the paths Axelstra added (recorded in `settings.json`), never exclusions the user
  or the organization set. The elevated step re-reads Defender afterwards; a refused change (Tamper Protection, policy) is
  reported as an error, never as success.
- The automatic part (indexer flags, `.metadata_never_index`) only stops content indexing; it does not lower antivirus protection.

## Limits of the agent role

The daemon tells the app from an agent by the connecting process (inside a session's Job Object on Windows, descendant
of the daemon on Unix), not by a secret. This is not a sandbox. Processes started outside the session's tree can still
read the token file and act as the user: on Windows through WMI, COM or Task Scheduler (`schtasks`), on Linux through
`systemd-run --user`, `at` or `cron`, and on macOS there is no tree check. A hostile agent that knows these techniques
can call the methods that `agent_forbidden` withholds from agents. See `docs/s3-app-secret.md`.

## Terminals started by the webview

`session.spawn` from the webview is limited by the host to a known shell name (compared without directory or
extension), and to `AXELSTRA_*` environment variables. It does not limit the shell's path or arguments: an agent
terminal is a shell started with `-Command <line>` / `-c <line>`, so running arbitrary commands is how the feature works.
A script injected into the webview could therefore run commands as the user. This is accepted by design; the barrier
against it is the Content Security Policy above and the fixed set of allowed methods, not this check.
