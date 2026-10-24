# Contributing

Build, test and layout notes are in [AGENTS.md](AGENTS.md); read it first.

## Before you open a pull request

- `cargo fmt --all --check` and `cargo clippy --workspace --all-targets -- -D warnings` are clean.
- `cargo test -p axelstra-core -p axelstra-daemon -p axelstra` passes.
- In `app/`: `npx tsc --noEmit` and `npm test` pass.
- `node scripts/stamp-version.mjs --check` passes (the three version fields agree).
- Every fix comes with a regression test.
- Commit subjects are short and in the imperative; the release notes are built from them.

## Versions

Do not bump versions by hand in a feature PR. The release workflow stamps the tag (`vX.Y.Z`) into
`Cargo.toml`, `app/package.json` and `app/src-tauri/tauri.conf.json`.

## License

The repository has no license file yet. Do not assume one; ask the maintainer before reusing code.
