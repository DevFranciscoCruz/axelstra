# Scan exclusions: design notes

API and shapes: `docs/SCANX-api.md`. Policy and risk: `SECURITY.md` ("Antivirus exclusions of the worktree folders").

## Today

- Automatic and harmless: indexer flags on `.axelstra-wt` (Windows `NOT_CONTENT_INDEXED`, macOS `.metadata_never_index`).
- Opt-in, per click, elevated: Defender `ExclusionPath` for each project's `.axelstra-wt` folder. One path per project parent, so
  N projects in different parents mean N exclusions (and one UAC prompt for all of them).
- UI: Settings > Desempenho (`PerformanceSection.tsx`) and a one-time dismissible notice (`ScanxHint.tsx`) that only opens it.

## Future option, not implemented: one dedicated worktrees folder

Worktrees live next to each project (`<parent>/.axelstra-wt`). A single dedicated directory would let one exclusion (or one
Dev Drive) cover every project:

- `%LOCALAPPDATA%\axelstra\wt` (macOS `~/Library/Application Support/axelstra/wt`, Linux `$XDG_DATA_HOME/axelstra/wt`), or a
  folder the user picks on a Dev Drive (ReFS, which already has Defender's lighter performance mode and needs no exclusion).
- Would need a `wt_dir` setting, a move of existing worktrees (`git worktree move`, janitor and sessions aware), updated
  path-length handling on Windows, and the `.axelstra-wt` guard of `scanx.bad_path` extended to that folder.
- Not migrated: existing worktrees stay where they are; there is no `wt_dir` setting yet.

## Linux

Nothing is changed. Desktop indexers (GNOME `tracker`/`localsearch`, KDE `baloo`) can be told to skip a folder in their own
settings (`tracker3` ignored directories, `balooctl config add excludeFolders`). The Desempenho section shows a short note
and no controls. A future step could drop a `.trackerignore` / call `balooctl`, but both are per-user tools we cannot verify here.
