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
- **Style attributes: `style-src-attr 'unsafe-inline'` remains.** Solid compiles static `style={{...}}` objects in JSX
  into `style="..."` attributes inside its template HTML, and nonces and hashes cannot cover attributes (only
  `'unsafe-hashes'` per exact value). About 40 static style objects in `app/src` would have to become classes or
  dynamic expressions before this can go. Attribute styles cannot define rules or `@import`, so the remaining
  exposure is small; removing it is tracked in the roadmap, not blocked by the platform.
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

