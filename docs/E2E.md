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
- On Windows also set `AXELSTRA_TEST_PIPE` to a unique value: the pipe name is `\.\pipe\axelstra-<SID>` plus `-t<value>`, so otherwise
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
node tools/e2e.mjs [path\to\axelstra-app.exe]
```

`--features tauri/custom-protocol` makes the debug host load `../dist` through `tauri.localhost` with the
production `csp` instead of the dev server and `devCsp`, so the check covers what users get. No release build is needed.

How it drives the app: the script sets `AXELSTRA_TEST_DEVTOOLS_PORT=N` and the **debug** host (`create_main_window` in `app/src-tauri/src/main.rs`; the window is created in code, `create: false` in `tauri.conf.json`) passes `--remote-debugging-port=N` (plus `AXELSTRA_TEST_BROWSER_ARGS`) to WebView2 as `additional_browser_args`. Reason: wry always sets its own default browser arguments through the API, and on runtime 155 (the CI runner) that made `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS` be ignored (the browser started without the port). The env var is still set as a fallback. Release builds never read the variables, so production has no debugging port. The failure report prints the effective env and the `Policies\Microsoft\Edge` registry keys. `N` is a port the OS just let the script bind (not one derived from the pid, which can fall in a Windows excluded range on the runners, where the browser cannot listen). The script launches the app with a throwaway `LOCALAPPDATA` (data dir), a unique
`USERNAME` (pipe name) and a throwaway WebView2 profile, attaches to the page over a WebSocket (Node's built-in
client, no dependencies) and evaluates DOM queries. Do not override `USERPROFILE`: the app exits at startup with it.
Set `E2E_VERBOSE=1` to see the app's stderr.

Scenarios it covers: **boot** (frontend renders, document title), **session** (a git repo is added with
`axelstra add-project`, a terminal is started with `axelstra terminal`, the UI lists the project, mounts an xterm
terminal with a sized WebGL canvas, and `axelstra read` shows the command ran), **restore** (the app process is
killed, the daemon keeps the session, a relaunch shows the terminal again), **addproject** (below). Throughout, it fails on any CSP violation
(Audits `ContentSecurityPolicyIssue`), console error or uncaught exception once the page has settled.
The CSP check is what verifies a `style-src` change (see SECURITY.md, "Content Security Policy").

### Scenario addproject (the Add project dialog)

Runs after `restore`, in the relaunched app. It opens _+ Adicionar projeto_ and checks, in this order: the six origins are listed;
`Home`, `ArrowDown`, `End` and `ArrowUp` (real `Input.dispatchKeyEvent`) move and select the tab; **Novo projeto** refuses a name with a
separator in the page, then creates `newparent/e2e-new` with a README and exactly one commit and the project shows in `axelstra projects`;
the dialog reopens on the origin used last; **Clonar** refuses `file://` in the page, then clones `ssh://127.0.0.1:1/none.git` (refused at
once, offline) and must show _A clonagem falhou_ and leave no `none` folder; **GitHub** shows its hint (no `gh` login, no list); **Descobertos**
shows a list or the empty note; `Esc` closes; no console error, CSP violation or uncaught exception meanwhile.

How it stays hermetic:

- Folders are typed into the path fields through the native value setter plus an `input` event; the native folder picker (`pick_folder`) is never
  clicked, so no OS dialog opens and nothing needs a test hook.
- The git identity comes from a throwaway `GIT_CONFIG_GLOBAL` (and `GIT_AUTHOR_*`/`GIT_COMMITTER_*`): `project.init` reads `git config user.name`, so
  the env vars alone are not enough. `GIT_TERMINAL_PROMPT=0` as well.
- `GH_CONFIG_DIR` is an empty temp dir and `GH_TOKEN`/`GITHUB_TOKEN` are empty: `gh` has no login, the tab shows its hint and no request leaves the machine.
- **Descobertos reads your real home.** The daemon resolves home from `USERPROFILE`, which the E2E cannot override (the app exits at startup with it), so the scan lists
  real repositories read-only. The scenario only checks that a list or the empty note renders and never adds from it; the refusal of a path the scan did not offer
  is covered by `crates/daemon/tests/sources.rs`; a successful add of a discovered repository has no automated test yet.
- **A positive clone is not driven through the page**: it only offers `https://`, `ssh://` and `user@host:path`, and there is no offline server for those. A clone from a
  local bare repo (`file://`, accepted by the daemon only under test) with progress, cancel and cleanup is covered by `crates/daemon/tests/sources.rs`.
  `tools/smoke.mjs` has no RPC client (the CLI has no raw call), so it does not cover `project.init`/`project.clone`.
- Never run it against ScorpioBots or any real project: every path it creates is under the temp dir of the run.

Cleanup stops only processes whose executable is the build under test, never the developer's own daemon.

Last local run (Windows 11, WebView2 155, debug host + custom-protocol): all scenarios passed.

## Driver options

| Option                                                        | Fits                                                                     | Notes                                                                                                                                                                                   |
| ------------------------------------------------------------- | ------------------------------------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| CDP straight to WebView2 (**used**, `tools/e2e.mjs`)          | boot, session, restore on Windows                                        | no driver binary, no msedgedriver version matching; Windows only                                                                                                                        |
| `tauri-driver` + WebDriver (WebdriverIO)                      | the same scenarios on Linux (WebKitWebDriver); Windows with msedgedriver | official; **no macOS**. Not set up: it adds two binaries (`tauri-driver`, a version-matched msedgedriver) for what CDP already does on Windows. Worth adding when a Linux E2E is wanted |
| Playwright against the Vite dev server with a mocked `invoke` | frontend flows only                                                      | fast and cross-OS, but does not exercise the Tauri host or the daemon                                                                                                                   |

## CI wiring

`ci.yml` job `e2e` (windows-latest, `continue-on-error: true`): builds the three pieces above and runs the script. It is
advisory: a red job does not fail CI or releases. Promote it to blocking once it has stayed green for two weeks.
The job first checks the WebView2 runtime in the registry and installs the Evergreen bootstrapper when it is missing (the
runner image does not promise it; without the runtime the app starts but opens no page, which showed as `no WebView2 page
target within 40s`). It runs with `E2E_VERBOSE=1`, and that failure lists the CDP targets seen, the runtime version, the data
dir report, the browser command lines (was the switch passed?), the listening sockets, the excluded port ranges, and the
profile's `chrome_debug.log` and `DevToolsActivePort`. Only CI confirms that the free-port change fixes the runner: if the
port is still not opened, that report names the cause.

## Not done yet

- Scenario 3 (update flow) and 4 (PROTO bump): no test config override for the updater endpoint exists, and no old daemon
  binary fixture exists.
- A Linux/macOS run. macOS has no WebDriver for WKWebView at all; Linux would need `tauri-driver` + xvfb.
- The fake agent is not used by `e2e.mjs` yet (it runs a plain shell command); switch the session scenario to
  `tools/fake-agent.*` when a scenario needs agent behaviour (done, conflict, hang).

## No window on the user's screen

`visual.mjs` and `e2e.mjs` start the app with `AXELSTRA_TEST_HIDDEN=1`. The Tauri host (`show_main_window` in
`app/src-tauri/src/main.rs`) then shows its window fully transparent (`WS_EX_LAYERED` alpha 0), click-through, a tool window and never activated, so nothing
appears on screen and no focus is stolen, while WebView2 keeps rendering (`Page.captureScreenshot` over CDP works, never a
screen grab). Both scripts use a temp data dir, a test pipe and a
private WebView2 profile, and kill the app's whole process tree in `finally`, also on failure. `smoke.mjs` runs only the
daemon, no window.

In hidden mode the host shows the window with `SW_SHOWNOACTIVATE` (never activated, never focused; an off-screen or minimised window is occluded and WebView2 would not render) and `e2e.mjs`
compares `GetForegroundWindow()` before and after: if the test changed it, the run fails with "stole the focus". The
harness only ever touches its own app process (`taskkill /T` on its pid), never the user's windows. Resizes go through the host window (`resizeClient` in `tools/winrect.mjs`, `SetWindowPos` with `SWP_NOACTIVATE`), never
through CDP `Browser.setWindowBounds`: in WebView2 that does not resize the host HWND, it moves the inner `Chrome_WidgetWin_1`
child by the browser-window position on every call (measured rel 8,31 -> 120,166 -> 232,301), so the page paints displaced
(an empty strip at the top and left, the right and bottom cut off). The host window and the WRY_WEBVIEW child stay correct, so
this was a test-driver bug, not a production one.

## Geometry check

`e2e.mjs` scenario **geometry** compares, at 100 %, after two real resizes and at 125 % and 150 %: `innerWidth/innerHeight x
devicePixelRatio` against the Win32 client area of the app window, `#root`/`html`/`visualViewport` at 0,0 filling the
viewport, no scroll offset, and the webview's child windows (`layoutProblems` in `tools/winrect.mjs`) at the same offset and
size (1 px rounding allowed) as the `WRY_WEBVIEW` child. 125 % and 150 % relaunch the app with WebView2
`--force-device-scale-factor` (the `launch(scale)` argument), which changes the page's device pixel ratio without touching
the user's display; the system DPI of the host window is not changed, so a real per-monitor DPI move is not covered.

## Opening an .html path from the terminal

`tools/e2e-html-link.mjs` (same build and isolation as `e2e.mjs`, hidden window, temp data dir) checks that Ctrl+click (Cmd on
macOS) on an `.html`/`.htm` path printed in a terminal opens it in the browser. The host is started with
`AXELSTRA_TEST_OPENER_LOG=<temp file>`: instead of calling the system opener it appends the resolved `file://` url to that file,
so no browser opens. A session prints `page.html` (inside its cwd), `notes.txt` and `..\outside\x.html`; real mouse events over
CDP (`Input.dispatchMouseEvent`, `modifiers: 2` for Ctrl) then assert that Ctrl+click on `page.html` logs exactly its url, and that
a plain click, `notes.txt` and the path outside the cwd (refused by `files.resolve_html`) log nothing. The terminal buffer is not
reachable from the page, so the script finds the row by trying each row of the screen. Not run in CI (it needs the host's
`open_html_file` command and the daemon's `files.resolve_html`).

## Kanban drag and drop

`tools/e2e-kanban.mjs` (same build and isolation as `e2e.mjs`, hidden window, temp data dir) writes four cards as `.kanban/*.md` in a
throwaway git repo, opens the Kanban tab and drives it with real mouse and key events over CDP: a pointer drag to another
column, a reorder inside a column (kept in the cards' `priority`, so it survives a reload), Esc and a drop outside the columns
both cancel, and Space/ArrowRight/Space moves a card by keyboard with the live region filled and focus kept. It reads the card
files back to check what was persisted.
