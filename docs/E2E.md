# E2E smoke plan

Goal: catch "the app does not boot / does not restore / cannot update" before a release. The CLI-level smoke
(`tools/smoke.mjs`) and the app-level E2E (`tools/e2e.mjs`) both run in CI; the updater and PROTO scenarios are still plans.

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

## Running it: `tools/e2e.mjs` (Windows)

```
cargo build -p axelstra
(cd app && npm ci && npm run build)
cargo build -p axelstra-app --features tauri/custom-protocol   # production CSP + bundled frontend, still a debug profile
node tools/e2e.mjs [path	oxelstra-app.exe]
```

`--features tauri/custom-protocol` makes the debug host load `../dist` through `tauri.localhost` with the
production `csp` instead of the dev server and `devCsp`, so the check covers what users get. No release build is needed.

How it drives the app: WebView2 opens a Chrome DevTools Protocol port when `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS`
holds `--remote-debugging-port=N`. The script launches the app with a throwaway `LOCALAPPDATA` (data dir), a unique
`USERNAME` (pipe name) and a throwaway WebView2 profile, attaches to the page over a WebSocket (Node's built-in
client, no dependencies) and evaluates DOM queries. Do not override `USERPROFILE`: the app exits at startup with it.
Set `E2E_VERBOSE=1` to see the app's stderr.

Scenarios it covers: **boot** (frontend renders, document title), **session** (a git repo is added with
`axelstra add-project`, a terminal is started with `axelstra terminal`, the UI lists the project, mounts an xterm
terminal with a sized WebGL canvas, and `axelstra read` shows the command ran), **restore** (the app process is
killed, the daemon keeps the session, a relaunch shows the terminal again). Throughout, it fails on any CSP violation
(Audits `ContentSecurityPolicyIssue`), console error or uncaught exception once the page has settled.
The CSP check is what verifies a `style-src` change (see SECURITY.md, "Content Security Policy").

Cleanup stops only processes whose executable is the build under test, never the developer's own daemon.

Last local run (Windows 11, WebView2 155, debug host + custom-protocol): all scenarios passed.

## Driver options

| Option | Fits | Notes |
| --- | --- | --- |
| CDP straight to WebView2 (**used**, `tools/e2e.mjs`) | boot, session, restore on Windows | no driver binary, no msedgedriver version matching; Windows only |
| `tauri-driver` + WebDriver (WebdriverIO) | the same scenarios on Linux (WebKitWebDriver); Windows with msedgedriver | official; **no macOS**. Not set up: it adds two binaries (`tauri-driver`, a version-matched msedgedriver) for what CDP already does on Windows. Worth adding when a Linux E2E is wanted |
| Playwright against the Vite dev server with a mocked `invoke` | frontend flows only | fast and cross-OS, but does not exercise the Tauri host or the daemon |

## CI wiring

`ci.yml` job `e2e` (windows-latest, `continue-on-error: true`): builds the three pieces above and runs the script. It is
advisory: a red job does not fail CI or releases. Promote it to blocking once it has stayed green for two weeks.

## Not done yet

- Scenario 3 (update flow) and 4 (PROTO bump): no test config override for the updater endpoint exists, and no old daemon
  binary fixture exists.
- A Linux/macOS run. macOS has no WebDriver for WKWebView at all; Linux would need `tauri-driver` + xvfb.
- The fake agent is not used by `e2e.mjs` yet (it runs a plain shell command); switch the session scenario to
  `tools/fake-agent.*` when a scenario needs agent behaviour (done, conflict, hang).
