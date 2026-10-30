# Releasing

How a release is cut, who may sign it, and what to do when a key is lost or leaked. The mechanics live in
`.github/workflows/release.yml`; the notes below are what the workflow cannot say.

## Cutting a release

1. Land everything on `main` and let CI go green.
2. Push a `vX.Y.Z` tag on a commit that is already on `main` (`git tag vX.Y.Z && git push origin vX.Y.Z`).
3. The workflow checks the tag is on `main`, runs the whole CI, stamps the tag into the three version files,
   builds, uploads to a **draft** release, merges `latest.json`, and only then publishes.
4. Approve the `release` Environment when GitHub asks (see below), and `release-publish` if it has a reviewer.

A manual run with *dry run* on does everything except publish: installers, `latest.json` and symbols stay on
the run as artifacts. A dry run also needs the `release` approval, because it uses the signing secrets.

## GitHub Environments

Signing secrets are **Environment** secrets, not repository secrets, so only jobs that name the Environment
can read them and the Environment's rules apply.

| Environment | Used by | Holds | Configure |
| --- | --- | --- | --- |
| `release` | the `release` build matrix | `TAURI_SIGNING_PRIVATE_KEY`, `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`, the six `APPLE_*` secrets, optional Azure/`TS_*` secrets | required reviewers (someone other than the tagger when possible), deployment tags limited to `v*` |
| `release-publish` | the final `publish` job | nothing | optional required reviewer: the last manual gate before clients can see the release |

Also protect the tag pattern `v*` (Settings, Rules, Tags): restrict who can create or delete `v*` tags, and
forbid updating them. Without that, anyone with write access can trigger a signing run.

Move the existing repository secrets into the `release` Environment, then delete them from the repository
level. Until the Environment exists GitHub creates it empty on first use, and the release fails at the
signing steps for lack of secrets.

### macOS is off until Apple signing exists

There is no Apple Developer account yet, so releases ship **Windows only** (plus the experimental Linux
files). The macOS legs of the `release` matrix are not even created unless the repository variable
`RELEASE_MACOS` is `true` (Settings, Secrets and variables, Actions, Variables), and `latest.json` lists only
the platforms that were built. To enable macOS: set `RELEASE_MACOS=true` and add the six `APPLE_*` secrets to
the `release` Environment. From then on the rule below applies, and a failure of the Apple silicon leg blocks
the publish (the Intel leg is experimental and does not).

Linux is experimental and never blocks a release: `publish` waits for it to finish and goes ahead whether it
succeeded or failed.

With macOS enabled, signing is mandatory. A real release **fails** on the macOS jobs when any of `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`,
`APPLE_SIGNING_IDENTITY`, `APPLE_ID`, `APPLE_PASSWORD`, `APPLE_TEAM_ID` is missing. There is no silent unsigned
Mac. A dry run only warns and builds unsigned, so the pipeline can be tested before the Apple account exists.
The setup of the Apple secrets is in the README ("Signing and notarizing the macOS app").

### Windows Authenticode (optional)

The Windows release can be Authenticode-signed: the `axelstra.exe` sidecar and `axelstra-app.exe` before the
installer packs them, then the NSIS installer. The provider comparison, the recommendation (SignPath
Foundation) and the owner's one-time setup are in `docs/SIGNING.md`. Unsigned builds trip machine-learning
antivirus engines more easily; reporting a false positive is in `docs/ANTIVIRUS.md`.

The workflow picks the provider from the `release` Environment secrets (step *Authenticode provider*):

| Provider | Secrets (`release` Environment) | Variables (optional) |
| --- | --- | --- |
| SignPath Foundation (preferred when both are set) | `SIGNPATH_API_TOKEN`, `SIGNPATH_ORGANIZATION_ID` | repository variables `SIGNPATH_PROJECT_SLUG` (default `axelstra`), `SIGNPATH_SIGNING_POLICY_SLUG` (default `release-signing`), `SIGNPATH_ARTIFACT_CONFIGURATION_SLUG` (default `windows`) |
| Azure Artifact Signing (formerly Trusted Signing) | `AZURE_TENANT_ID`, `AZURE_CLIENT_ID`, `AZURE_CLIENT_SECRET`, `TS_ENDPOINT`, `TS_ACCOUNT`, `TS_PROFILE` | none |

SignPath also needs `.signpath/artifact-configuration.xml` pasted into the SignPath project as an artifact
configuration (one configuration for both signing requests; owner checklist in `docs/SIGNING.md`).

**To turn it on:** add the secrets of one provider to the `release` Environment, then run the workflow by hand
with *dry run* on and check the *Verify the Authenticode signatures* step. With SignPath, approve the two
signing requests (binaries, then installer) in SignPath while the job waits; each waits up to 30 minutes, and
a dry run submits them too.

**Without the secrets** nothing changes: one `tauri build` as before, an unsigned installer, a notice in the
log. SmartScreen warns on first runs, and ML antivirus engines are more likely to flag the files.

**With a provider configured:**
- The build is split: `tauri build --no-bundle`, sign `axelstra-app.exe` and the sidecar
  (`app/src-tauri/binaries/axelstra-x86_64-pc-windows-msvc.exe`) and check them, `tauri bundle --bundles nsis`,
  sign the installer. `bundle.windows.signCommand` is not used because SignPath signs asynchronously with a
  human approval, which does not fit a per-file hook inside `tauri build`.
- Signing is fail-closed: a configured provider that fails, times out or is refused fails the release job, so
  nothing ships half signed. To ship unsigned on purpose, remove the secrets.
- *Verify the Authenticode signatures* requires `Get-AuthenticodeSignature` status `Valid` on the installer
  and on `axelstra-app.exe` and `axelstra.exe` read back out of it with 7-Zip (a warning only if 7-Zip cannot
  read a file out).
- The updater signature (`.sig`) covers the exact installer bytes. Signing changes them, so the workflow
  re-runs `tauri signer sign` on the signed installer. That step fails the job on error: a stale `.sig`
  would break updates for everyone.
- Not run yet with either provider: written without a SignPath project or an Azure account. Test with a dry
  run first.

## Release assets are immutable once published

Uploads use `gh release upload --clobber`, which is needed to re-run a failed job against its own draft. Every
upload step first checks that the release is still a **draft** and fails if it is already published, so a
published release can never have its assets replaced by a re-run.

## Updater key custody, escrow and rotation

The updater verifies every download against the **public** key in `app/src-tauri/tauri.conf.json`
(`plugins.updater.pubkey`). Whoever holds the matching **private** key can ship code to every installed copy,
and if the key is lost nobody can: installs can no longer update and must be reinstalled by hand.

**Custody**
- The private key and its password exist in exactly two places: the `release` Environment secrets and an
  offline escrow copy. Never in the repo, in a chat, or on a CI cache.
- The password is stored separately from the key.

**Escrow**
- Keep an encrypted copy of the key file (`age`/GPG or a password manager's vault) in storage the owner
  controls, plus the password in a different place, and a second person who can recover both.
- Test the escrow once a year: decrypt it and sign a dummy file with
  `npx tauri signer sign -f key.file -p <password> some-file`, then verify with the public key.

**Rotation (planned, no leak)**
The updater accepts only one public key, so a plain swap strands old installs. Rotate in two releases:
1. Generate the new pair (`npx tauri signer generate`). Ship release N **signed with the old key** whose app
   contains the **new** public key. Installs that update to N now trust the new key.
2. Move the new private key into the `release` Environment. Release N+1 and later are signed with it.
3. Users who skipped N cannot update past N+1 automatically: say so in the notes and link the installer.
4. Retire the old key only after N has been the latest release for a while. Destroy the old escrow copy.

**Compromise or loss**
- Leaked: delete the Environment secret immediately, then rotate as above. Because an attacker can sign
  updates until installs trust a new key, publish a manual installer and announce it; the in-app updater
  cannot repair this by itself.
- Lost: no automated recovery. Generate a new pair, ship a new installer and tell users to reinstall once.

## Debug symbols

`Cargo.toml` keeps `strip = true` for local builds. The release workflow overrides it per build with
`CARGO_PROFILE_RELEASE_DEBUG=line-tables-only`, `CARGO_PROFILE_RELEASE_STRIP=none` and
`CARGO_PROFILE_RELEASE_SPLIT_DEBUGINFO=packed`, so rustc emits `.pdb` files on Windows and `.dSYM` bundles on
macOS next to the binaries. They are uploaded as the workflow artifact `symbols-<platform>-<version>`
(retention 90 days). The shipped installers contain no debug info beyond what the platform keeps in the
binary: a panic backtrace from a user can be symbolicated with the matching artifact.

Workflow artifacts are readable by anyone who can read the repository. Symbols contain function names and
line tables, not secrets. If that ever matters, upload them to a private store instead.

On MSVC, `strip = true` makes rustc skip the `.pdb`, which is why CI does not use it. Not verified on a runner
yet: check the `symbols-*` artifact of the first dry run is not empty (the step only warns if it is).

## Build id and `PROTO`

Two different numbers decide whether the app replaces a running daemon:

- **`PROTO`** (`crates/daemon/src/proto.rs`) is the app-daemon *contract* version. It is bumped by hand, only
  when the app starts calling a daemon method an old daemon lacks (see `AGENTS.md`).
- The **build id** identifies the exact daemon binary (the release version plus the git commit it was built
  from). It changes on every release, with or without a protocol change.

A daemon outlives app updates. With `PROTO` alone, a release that only fixes daemon bugs leaves the old
daemon code running. The intended rule: the daemon reports its build id in the ping, and the app restarts a
daemon whose build id differs from its own, in addition to restarting one whose `PROTO` is older. `PROTO`
stays the hard compatibility floor; the build id is the freshness check.

### Version, commit and date

Separately from the build id, every binary carries `VERSION`, `COMMIT` and `BUILT` (`axelstra_core::buildinfo`).
`ci.yml` sets `AXELSTRA_GIT_SHA` from `github.sha` and `release.yml` adds a "Build metadata" step (sha from
`GITHUB_SHA`, date from `date -u`) before the sidecar and `tauri build` steps, in both the desktop and Linux jobs, so
the daemon sidecar and the app embed the same values even on a shallow checkout. `scripts/build-installer.ps1` does the
same locally. The daemon reports `version`, `commit` and `built` in `hello`/`ping`/`host.info` (additive, no `PROTO`
bump); the app compares them with its own and Settings > About warns when the running daemon differs (an old daemon
without the fields shows "unknown"). Its *Copy info* button gives users a block to paste in bug reports.

Status: this section documents the design. The code side (daemon ping field, app comparison, restart banner)
is a separate change; until it lands only `PROTO` triggers a restart.

## Entitlements and the sherpa libraries (macOS)

`app/src-tauri/entitlements.plist` no longer grants `com.apple.security.cs.disable-library-validation`.
That entitlement lets the app load dylibs signed by *any* team, which weakens the hardened runtime. It is not
needed because the sherpa speech engine is linked statically (`sherpa-rs` with the `static` feature): the
bundle holds no loose `.dylib`.

If a dylib ever has to ship in the bundle (a Tauri resource, a sidecar dependency), sign it with the same
Developer ID as the app, in the same release job, before the app is signed:

```sh
codesign --force --timestamp --options runtime --sign "$APPLE_SIGNING_IDENTITY" path/to/lib.dylib
```

The `Verify the macOS signature` step in `release.yml` runs `codesign --verify --deep --strict`, checks that
`disable-library-validation` is absent, and that every `.dylib`/`.so` in the app has the app's team ID. It
fails the release otherwise. `allow-jit` and `allow-unsigned-executable-memory` stay for WebKit; the second
is the first candidate for removal once a signed build can be smoke-tested on macOS.
Not verified on a real macOS machine from this repo yet.

## Release notes

`cliff.toml` configures [git-cliff](https://git-cliff.org). The `draft` job runs it (`--latest`) over the
commits since the previous `v*` tag and groups plain-English subjects by their first word (Add, Fix, Update...;
merge commits are skipped). If git-cliff fails or finds nothing, the job falls back to the plain list of commit
subjects. Preview locally with `git cliff --latest` (needs git-cliff installed).
The same text is the body of the update card in the app.

## Pinned actions

Actions are pinned by commit SHA with the tag in a comment. The signing actions were resolved on 2026-10-29:
`azure/artifact-signing-action` v2.0.0 (annotated tag, dereferenced to the commit) and
`signpath/github-action-submit-signing-request` v2. To move one, resolve the new tag
(`gh api repos/<owner>/<repo>/git/ref/tags/<tag>`, and for an annotated tag `gh api repos/<owner>/<repo>/git/tags/<sha> --jq .object.sha`).
