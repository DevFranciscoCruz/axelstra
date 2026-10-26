# Distribution

Platforms, signing status, and what users see. Release mechanics are in [RELEASING.md](RELEASING.md).

| Platform | Artifact | Updater key in `latest.json` | Status |
| --- | --- | --- | --- |
| Windows x64 | NSIS `-setup.exe` | `windows-x86_64` | supported; Authenticode optional |
| macOS Apple silicon | `.dmg`, `.app.tar.gz` | `darwin-aarch64` | **not available** until Apple signing exists; Developer ID + notarized (mandatory) once enabled |
| macOS Intel | `.dmg`, `.app.tar.gz` | `darwin-x86_64` | **not available**; experimental once enabled, see below |
| Windows arm64 | none | none | not built, see below |
| Linux | `.deb`, AppImage | none | experimental, no auto-update |

## macOS: Apple silicon and Intel

**Not released for now.** There is no Apple Developer account, so the macOS legs are skipped: they only run
when the repository variable `RELEASE_MACOS` is `true` and the `APPLE_*` secrets are set (see
[RELEASING.md](RELEASING.md)). Everything below describes the setup once that is enabled.

The release matrix builds each Mac architecture on a native runner (`macos-latest` is arm64,
`macos-15-intel` is x86_64), so there is no cross-compile and no universal binary. The Intel entry is marked
experimental: if it fails the release still goes out, and `latest.json` simply has no `darwin-x86_64`
entry, so Intel installs get no update until a release includes it. It has never run yet.

Tauri names the updater archive `Axelstra.app.tar.gz` for every architecture; the workflow renames it to
`Axelstra_<version>_<aarch64|x86_64>.app.tar.gz` so the two do not overwrite each other. The platform keys
are the ones the Tauri updater expects (`darwin-aarch64`, `darwin-x86_64`); `scripts/write-latest-json.mjs`
takes any key and needs no change.

Alternative if one download for every Mac is wanted: a universal build (`--target universal-apple-darwin`
with both Rust targets installed). It needs the static sherpa libraries for both architectures and doubles
the app size, and the updater would then publish the same archive under both keys. Not done: the per-arch
build is simpler and each download is smaller.

## Windows arm64

Not built. The static sherpa libraries come as prebuilt binaries per target and the debug-CRT workaround in
`app/src-tauri/build.rs` was written for x64; nobody has tried `aarch64-pc-windows-msvc`. Arm64 Windows runs
the x64 installer under emulation, which is the supported answer for now. Revisit when the project can use an
arm64 Windows runner and sherpa ships arm64 libraries.

## SmartScreen

An installer without Authenticode, or with a brand-new certificate, triggers Windows SmartScreen
("Windows protected your PC"). Tell users to click *More info*, then *Run anyway*. Signing through Azure Trusted
Signing (optional step in `release.yml`) removes the unknown-publisher label; reputation for the application
still builds up over downloads. On macOS the Developer ID signature plus notarization avoids the Gatekeeper
block; an unsigned dry-run build does not pass Gatekeeper and must not be shared.

## Rollback

Clients read `releases/latest/download/latest.json`, so the newest *published* release wins, and the updater
never downgrades.

- **Bad release, fix coming**: ship the fix as the next patch version. This is the normal path.
- **Stop the bleeding now**: turn the bad release back into a draft (`gh release edit vX.Y.Z --draft`).
  `releases/latest` then resolves to the previous published release and clients stop seeing the bad one.
  Installs that already updated stay on it until the fixed version ships.
- **Never** delete or reuse a tag, or edit assets of a published release; the workflow refuses to upload to a
  published release.
- A user stuck on a bad version installs the previous installer by hand from the Releases page.

## Beta channel

There is no beta channel. The updater has one endpoint (`releases/latest`), and GitHub's `latest` ignores
pre-releases, so marking a release as *pre-release* already keeps it away from everyone: enough to share a
build with testers by link.

If a real beta channel is needed, the smallest design is a second manifest: the beta app build points its
updater endpoint at a `beta.json` on a fixed `beta` release that the workflow rewrites for pre-release tags
(`vX.Y.Z-beta.N`). It needs a build-time config for the endpoint (a `--config` overlay), a channel switch in
the app settings, and a decision on the version stamp (`scripts/stamp-version.mjs` accepts only `X.Y.Z`
today). Not built.
