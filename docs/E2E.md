# E2E smoke plan

Goal: catch "the app does not boot / does not restore / cannot update" before a release. Today only
unit tests and the CLI-level smoke (`tools/smoke.mjs`, in CI) exist. This is the plan for the rest.

## Already in place

`tools/smoke.mjs` boots the engine binary in an isolated data dir, talks to the daemon over the real pipe or
socket, spawns and kills a session. It proves the engine boots on Ubuntu and Windows. It does not drive the UI.

## Fixture: fake agent

`tools/fake-agent.sh` / `.ps1` act as the agent so no model or login is needed. Behaviour is chosen by
`AXELSTRA_TASK` (done, conflict, hang, flaky). E2E tests configure the agent command to point at it. See `docs/TOOLS.md`.

## Isolation (required for every scenario)

- Point `LOCALAPPDATA` (Windows), `HOME` (macOS) or `XDG_DATA_HOME` (Linux) at a temp dir: `proto::data_dir()` reads them.
- On Windows also set `USERNAME` to a unique value: the pipe name is `\.\pipe\axelstra-<USERNAME>`, so otherwise
  the test talks to the developer's own daemon. `tools/smoke.mjs` shows the pattern.
- Use a fresh git repo in a temp dir as the project.

## Scenarios

1. **Boot.** Launch the app, wait for the main window and a daemon answering. Assert the window title, the project
   list renders and no `crash.log` appears in the data dir.
2. **Session restore.** Open a project, spawn a session running the fake agent (task `hang`), close the app window
   (the daemon keeps the session), relaunch, assert the same session is listed with its scrollback. Then kill the daemon
   and relaunch to assert the persisted layout comes back with a "session ended" state instead of a blank screen.
3. **Update flow.** Serve a local `latest.json` plus a signed test bundle (signed with a throwaway updater key whose
   public half is injected through a test config override), point the updater endpoint at it, assert the app offers
   the update, installs it and reports the new version. Needs the build to accept an endpoint override
   (a test-only config; not present yet).
4. **PROTO bump.** Start an old daemon binary, launch the new app, assert the daemon is restarted (see "Bumping PROTO" in AGENTS.md).

## Driver options

| Option | Fits | Notes |
| --- | --- | --- |
| `tauri-driver` + WebDriver (WebdriverIO) | scenarios 1, 2, 4 | official; Windows (msedgedriver) and Linux (WebKitWebDriver); **no macOS** support |
| Playwright against the Vite dev server with a mocked `invoke` | frontend flows only | fast and cross-OS, but does not exercise the Tauri host or the daemon |
| Playwright over CDP to WebView2 | Windows full-app | works with `--remote-debugging-port` through `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS`; Windows only |

Recommended: `tauri-driver` for 1, 2, 4 on Windows and Ubuntu (xvfb on Linux), against a debug build; scenario 3 on
Windows only, as a manual or nightly job because it needs a signed bundle.

## CI wiring

Separate workflow (`e2e.yml`, nightly and `workflow_dispatch`, not part of `ci.yml` so releases do not wait on a flaky
driver): build the app in debug, install `tauri-driver`, run the scenarios, upload `crash.log`, `lag.log` and screenshots
on failure. Promote single scenarios into `ci.yml` once they stay green for two weeks.

## Not done yet

Everything above except `tools/smoke.mjs`: no driver is installed, no test config override for the updater endpoint exists,
and no fixture project helper exists.
