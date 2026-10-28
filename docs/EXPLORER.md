# Explorer: local files without the daemon

A slow or overloaded daemon used to stall the file explorer ("A carregar... Está a demorar mais do que o normal."),
because every folder listing went `api.ts` -> Tauri `daemon_call` -> daemon pipe -> `server/files.rs`. A folder on this
disk does not need the daemon. The host (`app/src-tauri/src/localfs.rs`) now serves it itself through explicit commands.

## What the explorer calls today

| Frontend (`api.ts`) | Daemon method | Used by | Local command (new) |
|---|---|---|---|
| `fsList(worktree, path)` | `fs.list` -> `{entries: [{name, dir}]}` | `FilesPanel` | `local_fs_list` |
| `fsRead(worktree, path)` | `fs.read` -> `{text, eol, bom, hash}` | `Editor` | `local_fs_read` |
| `fsStat(worktree, path)` | `fs.stat` -> `{hash: string\|null}` | `Editor`, terminal links | `local_fs_stat` |
| `revealItemInDir(abs)` (opener plugin, from the webview) | none | `FilesPanel` menu | `local_fs_act` (`reveal`) |
| (none: no "open with default program" in the explorer yet) | none | | `local_fs_act` (`open`) |
| `fs.watch` events: `watch_set` -> daemon `watch.subscribe` -> `fs.changed` | | `fsTick` | `local_fs_watch` -> event `local-fs-changed` |
| `fsSearch`, `fsGrep` (`fs.search`, `fs.grep`) | | search box | stay on the daemon |
| `fsCreate`, `fsRename`, `fsDelete`, `fsWrite` (`fs.create/rename/delete/write`) | | menu, `Editor` save | stay on the daemon (writes; `check_params` narrows `fs.write`) |
| `resolvePath`, `pathAction`, `openHtmlFile` (`files.resolve_path`, `files.path_action`, `files.resolve_html`) | | terminal Ctrl+click | stay on the daemon: they need the session's cwd and meta |

`ALLOWED_METHODS` is unchanged: the daemon methods stay for remote hosts (`@host:` paths, ssh/wsl/container) and for
everything above that stays. No new daemon method, so no `PROTO` bump. The unit test that compares `ALLOWED_METHODS`
with the frontend still holds as long as the frontend keeps calling those methods for remote paths.

## Rule for the frontend

Use the local command when the worktree is local (`!isRemote(wt)`). Fall back to the daemon call when the local command
fails with code `localfs.not_local` (remote path, UNC or device path, empty). Every other `localfs.*`/`fs.*` code is the
final answer: show it (`errorText`), do not retry through the daemon.

## Commands

All are Tauri commands (`invoke`), failing with `{message, code?, params?}` like `daemon_call` (same `CallError`).
`worktree` is the root exactly as the page knows it (any slash style, trailing slash fine); `path` is relative to it
(`""` = the root). The page never sends an absolute path.

- `local_fs_list {req: u64, worktree, path, offset?: number, limit?: number, onChunk: Channel<Entry[]>}`
  -> `{entries: Entry[], total: number, truncated: boolean}`, `Entry = {name: string, dir: boolean}`.
  Same view as `fs.list` (`.gitignore` applied, `.git` hidden, folders first, then case-insensitive name).
  Runs on a blocking thread, never on the UI thread. `onChunk` receives raw entries (disk order, 256 at a time) while
  the folder is read, so the page can show rows before the sorted answer arrives; the answer replaces them.
  At most 20 000 entries are read (`truncated: true` when there were more); `total` = entries read; `offset`/`limit`
  slice the sorted result. `req` is a page-chosen id (a counter); reusing an id cancels the older listing.
- `local_fs_cancel {req}` -> void. Stops listing `req` (the listing then fails with `localfs.cancelled`; ignore it).
  A cancel that arrives before its listing starts is remembered. Call it when navigating away. A read blocked inside
  the OS (a dead network mount) cannot be interrupted, but it holds only its own worker thread.
- `local_fs_stat {worktree, path}` -> `string | null` (content hash, `null` when the file does not exist).
- `local_fs_read {worktree, path}` -> `{text, eol: "lf"|"crlf", bom: boolean, hash}` (UTF-8 text up to 2 MB, as `fs.read`).
- `local_fs_act {worktree, path, action: "open"|"reveal"}` -> void. `reveal` shows it in the file manager; `open` uses
  the default program, and only for a regular file that is not a program or script (same rules as terminal paths,
  `pathopen`; never runs anything; `run` is refused). Replaces the webview's own `revealItemInDir`, after which the
  `opener:allow-reveal-item-in-dir` capability can be dropped (kept for now; `scripts/check-tauri-config.mjs` is unchanged).
- `local_fs_watch {worktree: string | null}` -> void. One watch at a time (a new call replaces it, `null` stops it).
  Emits the Tauri event `local-fs-changed` with payload `{worktree}` once per debounced batch (listen with
  `@tauri-apps/api/event`, already allowed by `core:event:default`). It is the daemon's own watcher (`notify`, event
  driven, no polling, `.gitignore` aware) running in the host. The page should refresh the listed folders on it, as it
  does on `fsTick`. If starting fails the call rejects: keep the manual Refresh. The daemon's `fs.changed` stays for the
  rest of the app (git status etc.) and for remote worktrees.

## Security

The host serves only inside allowed roots, computed from disk on every call (no cache, no daemon):
the `path` of every project in `<data dir>/projects.json` (envelope or bare list, `.1`..`.3` backup when it does not
parse; read-only, the host never moves a file aside) plus every linked worktree (`<project>/.git/worktrees/*/gitdir`).
So a stuck daemon changes nothing. Per call:

1. `worktree` must equal a root (`paths::same`). Else `localfs.outside_roots`. Remote (`@host:`), empty, and on Windows
   UNC/device (`\\server\share`, `\\.\`, `\\?\UNC\`) -> `localfs.not_local`. `\\?\C:\x` is a plain local path.
2. `path` must be relative with normal components only: no `..`, no root or drive, no NUL (`localfs.invalid_path`).
   On Windows also: device names `CON PRN AUX NUL COM1-9 LPT1-9` (with any extension, trailing spaces; `localfs.reserved_name`),
   no `:` (alternate data streams), no trailing dot or space.
3. Root and target are canonicalized (long paths work: std adds the verbatim prefix); the target must stay under the
   canonical root, so a symlink or junction leaving it is `localfs.outside_roots`.

No fs plugin scope, no asset protocol, no new capability, CSP untouched; commands are listed explicitly in
`invoke_handler`. Opening never runs programs. Known limit: check then use (a link swapped between the canonicalize and
the read) is not closed, same as the daemon's `fsops`.

## Errors (`app/src/msg/files.ts`, pt, en, ptBR)

`localfs.not_local`, `localfs.outside_roots`, `localfs.invalid_path`, `localfs.reserved_name`,
`localfs.permission_denied {path}`, `localfs.not_found {path}` (folder deleted or moved), `localfs.not_dir`,
`localfs.too_big {mb}`, `localfs.cancelled`, `localfs.io_failed {path, detail}`, `localfs.invalid_action {action}`.
Also reused: `fs.not_file`, `fs.binary`, `fs.not_utf8`, `files.path_not_allowed {action, path}`, `files.path_not_found`.

## Tests

`cargo test -p axelstra-app localfs`: roots from a temp `projects.json`, listing, traversal, outside roots, remote/UNC,
reserved names, symlink escape (skipped without the privilege), 130-file folder with cap, chunks and paging, cancel,
read/stat shapes, open/reveal rules. Temp dirs only; the opener itself is never invoked.
