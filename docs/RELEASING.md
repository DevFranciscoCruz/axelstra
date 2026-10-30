# Releasing

How a release is cut, who may sign it, and what to do when a key is lost or leaked. The mechanics live in
`.github/workflows/release.yml`; the notes below are what the workflow cannot say.

## Versioning: the version follows the content

The version is not bumped by hand and not one patch per push. [release-please](https://github.com/googleapis/release-please)
reads the [Conventional Commit](https://www.conventionalcommits.org) subjects merged since the last release and picks
the next version from what they contain (`release-please-config.json`):

| Commits since the last release | Bump | Example from 0.1.42 |
| --- | --- | --- |
| a breaking change (`feat!:`, `fix!:`, a `BREAKING CHANGE:` footer) | major; **minor while the major is 0** (`bump-minor-pre-major`) | 0.2.0 |
| at least one `feat` | minor | 0.2.0 |
| only `fix`, `perf`, `refactor`, `build`, `ops` | patch | 0.1.43 |
| only `docs`, `test`, `ci`, `chore`, `style`, or non-conventional subjects | no release | none |

1.0.0 is the owner's decision: add a `Release-As: 1.0.0` footer to a commit on `main` (or edit the release PR) and
release-please uses it.

The tree carries the released version: the three fields (Cargo `[workspace.package]`, `app/package.json`,
`app/src-tauri/tauri.conf.json`) and the workspace packages in `Cargo.lock` all say the last release (seeded at
0.1.42, `.release-please-manifest.json`). `node scripts/stamp-version.mjs --check` keeps them in step (CI).

`node scripts/next-version.mjs` prints the version the next release gets, with the same rules (it reads the
changelog sections of `release-please-config.json`); `--json` adds the counts per type. At the switch to
release-please there were, since v0.1.42, 28 `feat`, 23 `fix` and 10 `perf` commits: the first release is **0.2.0**.

## Cutting a release

1. Land work on `main` with Conventional Commit subjects (PR titles are checked by `pr-title.yml`; see `AGENTS.md`).
2. Every push to `main` runs `release-please.yml`, which keeps **one** release PR open, titled
   `chore(main): release X.Y.Z`. It carries the version in the three files, `CHANGELOG.md` and the manifest; the
   workflow adds a commit that syncs `Cargo.lock` (release-please cannot edit it) and dispatches `ci.yml` on the PR
   branch, so the PR has CI checks. The PR grows with each push: nothing is released until it is merged.
3. When a release is wanted, review the PR (edit the changelog text in it if needed) and **merge** it.
4. The next `release-please.yml` run (the merge is a push to `main`) creates the tag `vX.Y.Z` and a GitHub
   Release whose body is the changelog section, turns that release back into a **draft** at once, and dispatches
   `release.yml` on the tag ref with `dry_run=false`.
5. `release.yml` checks the tag is on `main`, runs the whole CI, stamps the tag into the version files (a no-op:
   they already agree), builds and signs, uploads to the draft, merges `latest.json`, and only then publishes.
6. Approve the `release` Environment when GitHub asks (see below), and `release-publish` if it has a reviewer.

Why the dispatch: a tag or a release created with `GITHUB_TOKEN` starts no `push` or `release` workflow (GitHub's
loop guard), so the `on: push: tags` trigger never fires for release-please's tag. `workflow_dispatch` is the
exception to that rule. A `vX.Y.Z` tag pushed by hand still starts `release.yml` through `push`, as before; only do
that for a version release-please did not cut.

Why the draft: release-please publishes its release at once, but the app reads
`releases/latest/download/latest.json`, which exists only after `release.yml`. The draft keeps the release out of
`latest` until its installers and `latest.json` are on it, and keeps the "assets are immutable once published" rule.
`release.yml`'s `draft` job keeps release-please's body and only falls back to git-cliff when the body is empty.

If the dispatch failed (the step is red), run it by hand: Actions, *release*, *Run workflow*, *Use workflow from*
the tag `vX.Y.Z`, *dry run* off. A real run on anything but a `vX.Y.Z` tag is refused by the `gate` job.

A manual run with *dry run* on does everything except publish: installers, `latest.json` and symbols stay on
the run as artifacts. A dry run also needs the `release` approval, because it uses the signing secrets.

## Local installers

`scripts/build-installer.ps1` without `-Version` stamps the build with the next release version
(`next-version.mjs --local`): the version the release PR would ship, or the last release plus a patch when nothing
releasable landed. When the installed Axelstra (the NSIS uninstall entry in `HKCU`/`HKLM`) is already as high, it
takes one patch above the installed one, so the local installer always upgrades the installed copy. The four files
that carry the version (`Cargo.toml`, `Cargo.lock`, `app/package.json`, `app/src-tauri/tauri.conf.json`) are
stamped for the build and their exact previous bytes are written back at the end, also when the build fails.
`-Version X.Y.Z` forces a version. A local build numbered like a future release (for example 0.2.0 before 0.2.0
ships) is not replaced by the updater when that release comes out with the same number; the next one replaces it.

## GitHub settings for release-please (owner, one time)

- **Settings, Actions, General, Workflow permissions:** tick *Allow GitHub Actions to create and approve pull
  requests*. Without it release-please cannot open the release PR. The default token permission can stay
  *read*: each job asks for what it needs.
- **Branch protection / ruleset on `main`:** if status checks are required, require the `ci.yml` job names (they
  run on the release PR through the dispatch); do not require *conventional commit title* (`pr-title.yml` gets no
  run on a PR opened by `GITHUB_TOKEN`), or the release PR can never merge. Allow squash merges (the PR title becomes
  the commit subject).
- **Tag rules (`v*`):** the ruleset that restricts who may create `v*` tags must let **GitHub Actions** create them
  (add it to the bypass list), or release-please cannot tag the release.
- **Environment `release`:** its *deployment branches and tags* rule must allow the tag pattern `v*`; a dispatched
  run on the tag ref is a tag deployment, as before.
- **Labels:** release-please creates `autorelease: pending` and `autorelease: tagged` itself; do not delete them
  while a release PR is open.

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

release-please writes the notes: the section of `CHANGELOG.md` for the version, grouped by commit type
(`changelog-sections` in `release-please-config.json`; docs, tests, CI, chores and style are hidden), becomes the
body of the GitHub Release. The `manifest` job copies that body into `latest.json`, so it is also the text of the
update card in the app.

Fallback, for a release whose draft has an empty body (a tag pushed by hand): `cliff.toml` configures
[git-cliff](https://git-cliff.org). The `draft` job runs it (`--latest`) over the commits since the previous `v*` tag
and groups subjects by their first word (merge commits are skipped). If git-cliff fails or finds nothing, the job
falls back to the plain list of commit subjects. Preview locally with `git cliff --latest`.

## Pinned actions

Actions are pinned by commit SHA with the tag in a comment. The signing actions were resolved on 2026-10-29:
`azure/artifact-signing-action` v2.0.0 (annotated tag, dereferenced to the commit) and
`signpath/github-action-submit-signing-request` v2. To move one, resolve the new tag
(`gh api repos/<owner>/<repo>/git/ref/tags/<tag>`, and for an annotated tag `gh api repos/<owner>/<repo>/git/tags/<sha> --jq .object.sha`).
