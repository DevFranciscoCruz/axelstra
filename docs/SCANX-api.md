# scanx.* API (antivirus and indexer exclusions of `.axelstra-wt`)

For the frontend. Logic: `crates/daemon/src/scanx.rs`, RPCs: `crates/daemon/src/server/scanx.rs`. PROTO 31.
All methods are **app only** (`agent_forbidden`: every `scanx.*`), listed in `ALLOWED_METHODS`, wrapped in `app/src/api.ts`
(`scanxStatus`, `scanxDefenderApply`, `scanxDefenderRevert`, `scanxDiag`, `scanxHintDismiss`; types `ScanxStatus`, ...).

## What happens without the user doing anything

| OS | What | When |
| --- | --- | --- |
| Windows | `FILE_ATTRIBUTE_NOT_CONTENT_INDEXED` on `.axelstra-wt` and on every worktree folder (NTFS gives it to everything created below a flagged folder, checked on Windows 11). Existing content is flagged by a low-priority thread (cap 50 000 changes per tree, links skipped). | the folder is created (`worktree.create`, `worktree.from_pr`, any `create_at`), an orchestrator run starts, daemon start |
| macOS | empty `.metadata_never_index` in `.axelstra-wt` | same |
| Linux | nothing | - |

No admin, no PowerShell, no polling. Never touches Defender.

## Methods

### `scanx.status {}`

Slow (one PowerShell call, about a second): runs in the slow-read lane, so it may answer `{"loading": true}` after 20 s.

```json
{
  "os": "windows" | "macos" | "linux",
  "defender": {
    "state": "active" | "inactive" | "unavailable" | "n/a",
    "realtime": true, "tamper": true,
    "exclusions_readable": false
  },
  "folders": [{
    "project": "p1", "name": "proj", "path": "C:\\Users\\x\\Projetos\\.axelstra-wt",
    "exists": true,
    "indexable": true,            // false = flagged / marker present; null = unknown (Linux)
    "excluded": "yes" | "no" | "unknown" | "n/a",
    "source": "defender" | "recorded" | null,
    "drive": "C", "fs": "NTFS", "dev_drive": false
  }],
  "can_apply": true, "can_revert": false,
  "hint": false, "hint_dismissed": false,
  "job": {"running": null, "last": null},
  "error": "...", "error_code": "scanx.powershell_failed", "error_params": {}   // only when the query failed
}
```

* `state`: `active` = Defender is the antivirus doing real-time scanning; `inactive` = Defender is there but another product
  (or nothing) scans; `unavailable` = PowerShell or the Defender cmdlets did not answer; `n/a` = not Windows.
* **A non-admin process cannot read Defender's exclusions** (`Get-MpPreference` answers `N/A: Must be an administrator to view
  exclusions`, measured on Windows 11). So `excluded` is `"unknown"` unless the daemon runs elevated, or we added the path
  ourselves (`"yes"`, `source: "recorded"`, kept in `settings.json` under `scanx.applied`). The UI should say "applied by
  Axelstra" for `recorded`, and offer Apply when `unknown`/`no`.
* `dev_drive`: the volume is ReFS (Dev Drive's file system). The hint is never raised there.
* `tamper: true` means Tamper Protection is on; an apply may be refused (`scanx.defender_blocked`), tell the user so.
* Several projects can share one folder (same parent): their rows repeat the same `path`.

### `scanx.defender_apply {}` / `scanx.defender_revert {}`

Answer at once: `{"started": true, "op": "apply" | "revert"}`, or a coded error (`scanx.unsupported` off Windows,
`scanx.busy` when one is already waiting). The elevated PowerShell (UAC prompt) runs on a daemon thread; the result is the
watch-stream event `scanx.changed` (add `"scanx.changed"` handling with `onDaemonEvent`):

```json
{"event": "scanx.changed", "op": "apply", "ok": true, "changed": ["C:\\...\\.axelstra-wt"], "unchanged": []}
{"event": "scanx.changed", "op": "revert", "ok": false, "error": "English text", "error_code": "scanx.elevation_cancelled", "error_params": {}}
```

and stays in `scanx.status.job.last`. Rules:

* Apply adds `Add-MpPreference -ExclusionPath` for the `.axelstra-wt` folders of the registered projects and nothing else (every
  path is checked to be a `.axelstra-wt` folder; never a drive or a process). A path already excluded is skipped (`unchanged`),
  so applying twice never duplicates. A successful apply also dismisses the notice.
* Revert removes exactly those paths (plus any recorded one whose project is gone), nothing the user added.
* After the change the elevated script reads the exclusions again; a path Defender silently ignored is an error
  (`scanx.defender_blocked`), never a success.
* Both write an audit entry (`scanx.defender_apply` / `scanx.defender_revert`, session 0).
* Never call them without the user's click.

### `scanx.diag {}`

`{"os": "windows", "state": "ok", "procs": [{"name": "MsMpEng", "pid": 4, "cpu": 64, "rss": 685256704}]}` for `MsMpEng`,
`SearchIndexer`, `SearchProtocolHost` (`cpu` = percent of one core now, `rss` = working set in bytes; one reading per call, no
polling). Off Windows: `{"os": "...", "state": "n/a", "procs": []}`. Slow-read lane like `status`.

### `scanx.hint_dismiss {}`

`{}`. Stores `hint_dismissed` in settings (`scanx` key) and clears the pending notice; also emits `scanx.changed {op: "hint"}`.

## The one-time notice

`status.hint` is true when the notice is pending and not dismissed. It becomes pending (once in the life of the settings
file, `scanx.hint_shown`) when `.axelstra-wt` is created for the first time **and** Defender is the active antivirus **and** the
volume is not ReFS/Dev Drive **and** the folder is not already excluded. The daemon then emits `scanx.changed {op: "hint"}`.
Not raised for folders that existed before this version.

## Message codes (`app/src/msg/scanx.ts`)

`scanx.unsupported`, `scanx.no_folders`, `scanx.too_many_paths {count}`, `scanx.bad_path {path}`, `scanx.powershell_failed {detail}`,
`scanx.elevation_cancelled`, `scanx.elevation_failed {detail}`, `scanx.defender_blocked {detail}`, `scanx.defender_failed {detail}`,
`scanx.busy`.

## Not done

* Windows Search crawl-scope rule (`ISearchCrawlScopeManager`): needs COM interop (the `windows` crate is not a dependency) and
  cannot be proven without a real machine; the content-indexed attribute already stops content indexing. Skipped on purpose.
* Worktree location is not migrated (no `wt_dir` setting).
