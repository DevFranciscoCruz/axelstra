# Platform support

What works on which OS, and where it does not. Audit of commit `72054b4`, updated at `8aa82d9`; line numbers drift as
the gaps close. "Full" means the same behavior as the reference OS for that row, not byte-identical code.

Status: **Windows** is the reference (shipped, signed, E2E). **Linux** and **macOS** build and pass the engine tests in
CI (`ci.yml` `test` matrix); the release matrix builds macOS only when `RELEASE_MACOS` is set and Linux as an
experimental job.

## Feature table

| Feature | Windows | Linux | macOS | Gap |
|---|---|---|---|---|
| Daemon transport | named pipe, DACL on current user + SYSTEM, SID in name | unix socket in data dir | unix socket in data dir | none |
| Peer check (`origin_of`) | pid + `GetProcessTimes` | pid + `/proc/<pid>/stat` field 22 | pid + `proc_pidinfo` start (µs) | none |
| Pty and process ownership (`job.rs`) | Job Object, KILL_ON_JOB_CLOSE | process group per child, `waitid` | same as Linux | none |
| Orphan adoption (`adopt_orphans`) | not needed (Job Object) | `PR_SET_CHILD_SUBREAPER` | nothing: a double-forked child escapes its session | G4 |
| Process table (`procs.rs`) | Toolhelp + working set per pid, `pgid` = 0 | one `ps -A -o pid,ppid,pgid,rss` | same `ps` (columns accepted by BSD `ps`) | G2 (spawn) |
| Process names (`procs::names_of`) | Toolhelp `szExeFile` | `ps -o comm=` | `ps -o comm=`, full path with spaces parsed (`parse_ps_names`) | none |
| Memory level source | `GlobalMemoryStatusEx` + commit % | `/proc/meminfo` `MemAvailable` | `host_statistics64` + `hw.memsize` (`governor/mac.rs`, no spawn) | none |
| Memory events (`governor.rs` `os_source`) | `CreateMemoryResourceNotification` | PSI trigger + `poll` | none: computed deadline only (15 s cap) | G3 |
| Pressure cause (`governor/cause.rs`) | kernel pools, handles, pool tags, CPU/IO, compression | `meminfo` slab, cache, zswap, `/proc/stat`, per-pid cpu/io | `null` fields named with a reason in `breakdown.unavailable` | G5 (data) |
| Pause / resume (`suspend.rs`) | `NtSuspendProcess` | `SIGSTOP`/`SIGCONT` per pid | same as Linux | none |
| Indexer exclusion (`scanx.rs`) | `NOT_CONTENT_INDEXED` attribute | nothing (no indexer is in the way) | `.metadata_never_index` marker | none, by design |
| Defender hint (`server/governor.rs:196`) | exclusion command in the FMfn notice | n/a | n/a | none |
| Listening ports (`ports.rs`) | `GetExtendedTcpTable`, `netstat` fallback | `/proc/net/tcp{,6}` + `/proc/<pid>/fd`, `lsof` fallback | `lsof` | none |
| Default shell | `powershell -NoLogo` | `$SHELL -l`, else `/bin/bash` | `$SHELL -l`, else `/bin/zsh` | G7 |
| Agent launch line | PowerShell `& 'prog' ...` | `sh -l -c 'cmd; exec $SHELL -l'` | same | none |
| Agent CLI lookup (`agents.rs`) | `.exe .cmd .bat .com` | exec bit | exec bit | none |
| Hibernate wake | `-NoExit` PowerShell guard | `sh` `|| exit 97` | same | none |
| Open / reveal path (`pathopen.rs`) | `explorer.exe`, `ShellExecuteW` | `xdg-open` | `open` | none |
| Computer use (`computer.rs`) | Win32 | none (`computer.rs:505` stub) | `osascript` | G8 |
| File watcher | ReadDirectoryChanges | inotify | FSEvents, paths canonicalized | none |
| Atomic write | rename retry (AV locks) | rename + dir fsync | rename + dir fsync | none |
| Long paths | `\\?\` verbatim (`git.rs:306`) | n/a | n/a | none |
| Line endings | `.gitattributes`: LF everywhere, CRLF for `.ps1/.bat/.cmd` | LF | LF | none |
| App: usage (RSS, CPU, GPU) | RSS, CPU, GPU counters | RSS via `ps`; CPU, GPU `None` | RSS via `ps`; CPU, GPU `None` | G10 |
| Build jobs free memory (`buildenv::memory`) | `GlobalMemoryStatusEx` | `/proc/meminfo` | `governor::mac::memory_bytes` | none |
| App: OS toast with click (`notify_session`) | WinRT toast | plugin fallback | plugin fallback | G12 |
| App: keep awake (`stats.rs`) | `SetThreadExecutionState` | `systemd-inhibit` if present | `caffeinate` | none |
| App: GPU preference (`gpu.rs`) | registry GPU preference | n/a | n/a | none, by design |
| App: window `make_inert` (test-hidden window) | layered click-through | no-op | no-op | G13 |
| Statusline shell (`cli/statusline.rs`) | finds Git for Windows `bash.exe`, skips WSL stub | `bash` on PATH | `bash` on PATH | none |
| WSL / SSH hosts (`hosts.rs`) | `wsl -l`, ssh | ssh | ssh | none |
| Installer | NSIS (`build-installer.ps1`), signed, `installMode: passive` | `tauri.release.linux.conf.json`, experimental | `.dmg` when `RELEASE_MACOS=true` | G14 |
| Sidecar prep | `prepare-sidecar.ps1` | `prepare-sidecar.sh` | `prepare-sidecar.sh` | none |
| Updater manifest keys | `windows-x86_64` | none yet | `darwin-aarch64`, `darwin-x86_64` (experimental) | G14 |
| Docker / git / gh | CLI on PATH | CLI on PATH | CLI on PATH | none |

## Gaps

- **G1. No resident memory on Windows from the process table.** `crates/core/src/procs.rs:15-16,53` fills `rss_kib: 0` and
  `pgid: 0`. Anything that sums `rss_of_groups` (`procs.rs:34`) is 0 there; the Windows side gets memory from Job Objects
  (`job.rs`) and `win::process_stats` instead. Not a bug, but two code paths for one number.
- **G2. macOS process table.** `procs.rs:63` and `procs.rs:164` use `ps -A -o ...`; the `comm=` column is a full path on
  macOS, possibly with spaces, and `clean` (`procs.rs:138-141`) only splits on `/` and `\\`. A `ps` per call also costs a
  process spawn on every platform that has `/proc` or `libproc`. **Partly closed (`1409f40`):** Windows rows carry the working set (one `OpenProcess` per pid per snapshot, closed at once), the `comm=` parser keeps spaces. Still one `ps` spawn per snapshot on unix.
- **G3. macOS memory source has no event.** `governor.rs:329` `os_source` falls back to `Sampler` on macOS, so
  `has_events()` is false and the level is caught only by the deadline (`sample.rs`, `macos_now` runs `sysctl` and
  `vm_stat` per read). `commit_pct` is `None` (`sample.rs:79`). **Closed (`3221746`):** `governor/mac.rs` reads `host_statistics64` directly (no spawn). Still no OS event on macOS: the deadline is the only wake.
- **G4. No subreaper on macOS.** `server/origin.rs:119` `adopt_orphans` is `cfg(target_os = "linux")`. On macOS a
  process that double-forks out of its session group is no longer "descended from the daemon" (`origin.rs:111`), so
  `orphan_of_ours` misses it and the janitor/reaper may not see it.
- **G5. Cause breakdown is `null` on macOS.** `governor/cause.rs` (`linux_cpu`/`linux_io` stubs) returns `None` for per-pid CPU and I/O off
  Linux; `meminfo()` reads `/proc/meminfo`, which does not exist on macOS, so `cache_mb` and `compressed_mb` are
  `None`; the non-Windows `CauseSource` returns `None` for `kernel` and `system` unless Linux. The panel shows nothing without saying
  why. **Partly closed (`3221746`):** `breakdown.unavailable` names each `null` field and why; macOS cache/compressed/kernel data is still missing.
- **G6. Closed (`0a93b67`) on Linux.** Was: ports need `lsof` on unix. `crates/daemon/src/ports.rs:113`. Minimal Linux images (and containers) often
  lack it; `/proc/net/tcp` plus `/proc/<pid>/fd` would avoid the dependency. The error is reported (`tool_failed`), so
  the failure is visible.
- **G7. Shell choice is duplicated.** `crates/cli/src/ctl.rs:535` (`default_shell`) and the app each pick a shell;
  `automations.rs:190` and `hibernate.rs:141,209` switch on the program name containing `powershell`/`pwsh` and
  otherwise assume a POSIX shell. `fish`, `nu` or `csh` as `$SHELL` break `-c 'a; exec b -l'`. Windows never uses
  `pwsh` by default, only `powershell` 5.1.
- **G8. No computer use on Linux.** `crates/daemon/src/computer.rs:505` is a stub for everything but Windows and macOS.
- **G9. Closed (`8aa82d9`).** Was: macOS peer start time has 1 s resolution. `server/origin.rs:79` (`process_start`, `ps -o lstart=`). A pid reused within the
  same second after exit passes the pid + start check. Low risk; `libproc` `proc_pidinfo` gives microseconds.
- **G10. App usage has no CPU or GPU off Windows.** `app/src-tauri/src/stats.rs:342-346` return `None` / empty; memory
  uses a full `ps` dump (`stats.rs:323`).
- **G11. Closed (`8aa82d9`).** Was: `buildenv::memory` is `None` on macOS. `crates/daemon/src/buildenv.rs:223`. `CARGO_BUILD_JOBS` is then derived
  from cores alone, which can over-subscribe a small Mac. The governor's `vm_stat` reader (`sample.rs`) already has the
  data.
- **G12. No click-through OS toast off Windows.** `app/src-tauri/src/main.rs` (`notify_session`, :1035) returns `false`; the UI falls back
  to the notification plugin (`main.rs:1449`) which has no click callback, so clicking a notification on Linux/macOS does
  not focus the worktree.
- **G13. Test-hidden window is Windows only.** `main.rs:1407-1420` (`make_inert`). `AXELSTRA_TEST_HIDDEN`
  on Linux/macOS shows the window normally, so E2E (`tools/e2e.mjs`) is not hidden there.
- **G14. Release and updater are Windows first.** `release.yml:107-121` (matrix) builds macOS only with `RELEASE_MACOS`,
  Linux is the experimental job at `release.yml:346`, `tauri.conf.json:30` has `bundle.active: false` (bundling is done
  by the release config files), and `latest.json` has entries only for the legs that ran (`write-latest-json.mjs`). A
  Linux user gets no update. `tauri.conf.json:46` configures only the `windows` updater block.

## Hardcoded Windows assumptions that are safe

Checked and fine on other OS because they are behind `cfg!`/`cfg(windows)` or are only data for Windows:

- `.exe` names: `app/src-tauri/src/main.rs:143` (`cfg!(windows)`), `cli/statusline.rs:64,95,98,178`,
  `daemon/buildenv.rs:144` (tries both names), `daemon/fsops.rs:167` (VS Code shim, Windows path).
- Backslash handling: `core/paths.rs:13` (`norm` replaces `\\` only on Windows), `core/git.rs:306` (verbatim only on
  Windows), `daemon/hosts.rs:93` (`display`), `cli/ctl.rs:456-491` (strips `\\?\` prefixes; harmless elsewhere).
- Test code with `C:\wt\a` paths (`budget.rs`, `agentlog.rs`) tests string normalization and runs on every OS.

## Closing the gaps

Closed or partly closed on `ax/crossplat`: G1, G2 (`1409f40`), G3, G5 (`3221746`), G6 (`0a93b67`), G9, G11 (`8aa82d9`).
Open: G2 (unix spawn per snapshot), G4, G5 (macOS data), G7, G8, G10, G12, G13, G14. A `paused.json` written by an
older macOS daemon used the `ps` start hash, so after the update `recover_stale` leaves those pids alone (only after a
daemon crash with frozen sessions; a clean shutdown resumes everything).

Rule for every platform branch (from the task): a pure function with injected data and a unit test that runs on every
OS, plus a `cfg`-gated test for the real OS call.
