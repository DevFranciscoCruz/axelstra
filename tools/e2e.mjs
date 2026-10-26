// End-to-end smoke of the real desktop app (Windows, WebView2) driven over the Chrome DevTools Protocol.
//
//   cargo build -p axelstra
//   cargo build -p axelstra-app --features tauri/custom-protocol     (production CSP, bundled frontend: run `npm run build` in app/ first)
//   node tools/e2e.mjs [path\to\axelstra-app.exe]
//
// WebView2 exposes CDP when WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS carries --remote-debugging-port, which needs no
// driver binary and no msedgedriver version matching (see docs/E2E.md for why this replaces tauri-driver here).
// The app runs against a throwaway data dir and WebView2 profile, so it never touches the developer's own daemon.
//
// Scenarios: boot (window renders, no CSP violation, no uncaught error), session (a session spawned through the CLI
// shows up in the UI with a live terminal), addproject (the Add project dialog: origins by arrow keys, a new project, a
// clone that fails offline, GitHub without a login, discovered, Esc), removeproject (the sidebar X: list vs everything,
// warnings, typed name, counts), restore (the app is closed and relaunched: the
// daemon kept the session).
import { spawn, spawnSync } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { createServer } from "node:net";
import { realTmpdir } from "./lib/tmp.mjs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { layoutProblems, resizeClient, windowInfo } from "./winrect.mjs";

if (process.platform !== "win32") {
  console.log("e2e: WebView2 driver is Windows only; skipping");
  process.exit(0);
}
const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const exe = resolve(process.argv[2] ?? process.env.AXELSTRA_APP ?? join(root, "target", "debug", "axelstra-app.exe"));
const cliBin = join(dirname(exe), "axelstra.exe");
for (const f of [exe, cliBin]) {
  if (!existsSync(f)) {
    console.error(`e2e: missing ${f} (see the header of tools/e2e.mjs)`);
    process.exit(2);
  }
}

const dir = mkdtempSync(join(realTmpdir(), "axelstra-e2e-"));
const home = join(dir, "home");
mkdirSync(home, { recursive: true });
// A port the OS just let us bind: a pid-derived one can sit in a Windows excluded range (Hyper-V/WinNAT reservations on the
// runners), where the browser cannot listen and says so only in its own log.
const port = await new Promise((ok, no) => {
  const srv = createServer();
  srv.once("error", no);
  srv.listen(0, "127.0.0.1", () => {
    const { port: p } = srv.address();
    srv.close(() => ok(p));
  });
});
const user = `axe2e${process.pid}`;
// USERPROFILE stays real: overriding it makes WebView2 exit at startup. LOCALAPPDATA is what proto::data_dir() reads.
const env = {
  ...process.env,
  HOME: home,
  LOCALAPPDATA: home,
  AXELSTRA_DATA_DIR: join(home, "axelstra"), // explicit: the real data dir is never used (docs/TESTING.md)
  XDG_DATA_HOME: join(home, "share"),
  USERNAME: user,
  USER: user,
  AXELSTRA_TEST_PIPE: user, // the pipe is named by SID now, USERNAME no longer isolates it
  AXELSTRA_TEST_HIDDEN: process.env.AXELSTRA_TEST_HIDDEN ?? "1", // the host shows its window transparent, click-through, unfocusable
  WEBVIEW2_USER_DATA_FOLDER: join(dir, "webview"),
  // the host (debug builds) hands these to WebView2 itself; the env var alone is ignored by runtime 155 because wry sets its own args
  AXELSTRA_TEST_DEVTOOLS_PORT: String(port),
  WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port}`,
};
// A git identity for the daemon's `project.init` (it reads `git config user.name`), never the developer's own gitconfig.
writeFileSync(
  join(home, "gitconfig"),
  "[user]\n\tname = e2e\n\temail = e2e@example.invalid\n[init]\n\tdefaultBranch = main\n",
);
Object.assign(env, {
  GIT_CONFIG_GLOBAL: join(home, "gitconfig"),
  GIT_CONFIG_NOSYSTEM: "1",
  GIT_AUTHOR_NAME: "e2e",
  GIT_AUTHOR_EMAIL: "e2e@example.invalid",
  GIT_COMMITTER_NAME: "e2e",
  GIT_COMMITTER_EMAIL: "e2e@example.invalid",
  GIT_TERMINAL_PROMPT: "0",
  GH_CONFIG_DIR: join(home, "gh"), // no gh login in the throwaway profile: the GitHub tab shows its hint, no network call
  GH_TOKEN: "",
  GITHUB_TOKEN: "",
});
const cli = (...a) => spawnSync(cliBin, a, { env, encoding: "utf8", timeout: 20000 });
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
// without /T the daemon (a child of the app) survives, as it does when the user closes the window
const kill = (pid, tree = false) =>
  spawnSync("taskkill", ["/PID", String(pid), ...(tree ? ["/T"] : []), "/F"], {
    stdio: "ignore",
  });

/**
 * Foreground watcher: one long-lived PowerShell that prints a line each time the foreground window changes (and once at
 * the start): `FG <hwnd> <pid> <exe path>|<command line>`. The runner and other processes may change the foreground
 * (no interactive desktop, a job window, WebView2 helpers), so a plain before/after comparison is noise; what must never
 * happen is the app's own window (the app, the daemon, or a WebView2 process started with our debugging port) becoming
 * the foreground window at any sample.
 */
const FG_SCRIPT = `
Add-Type 'using System;using System.Text;using System.Runtime.InteropServices;public class FG{[DllImport("user32.dll")]public static extern IntPtr GetForegroundWindow();[DllImport("user32.dll")]public static extern uint GetWindowThreadProcessId(IntPtr h,out uint p);[DllImport("user32.dll",CharSet=CharSet.Unicode)]public static extern int GetClassName(IntPtr h,StringBuilder s,int n);[DllImport("user32.dll",CharSet=CharSet.Unicode)]public static extern int GetWindowText(IntPtr h,StringBuilder s,int n);[DllImport("user32.dll",EntryPoint="GetWindowLongPtrW")]public static extern IntPtr GetLong(IntPtr h,int i);}'
$last = -1
while ($true) {
  $h = [FG]::GetForegroundWindow().ToInt64()
  if ($h -ne $last) {
    $last = $h
    $p = [uint32]0
    [void][FG]::GetWindowThreadProcessId([IntPtr]$h, [ref]$p)
    $c = if ($p -ne 0) { Get-CimInstance Win32_Process -Filter "ProcessId=$p" -ErrorAction SilentlyContinue } else { $null }
    $cls = New-Object System.Text.StringBuilder 256
    $ttl = New-Object System.Text.StringBuilder 256
    [void][FG]::GetClassName([IntPtr]$h, $cls, 256)
    [void][FG]::GetWindowText([IntPtr]$h, $ttl, 256)
    $st = [FG]::GetLong([IntPtr]$h, -16).ToInt64()
    $ex = [FG]::GetLong([IntPtr]$h, -20).ToInt64()
    $line = "FG $h $p class=$cls title=[$ttl] style=0x$($st.ToString('x')) ex=0x$($ex.ToString('x')) $($c.ExecutablePath)|$($c.CommandLine)"
    [Console]::Out.WriteLine($line.Replace("\`r", ' ').Replace("\`n", ' '))
    [Console]::Out.Flush()
  }
  Start-Sleep -Milliseconds 100
}
`;
const fgSamples = [];
const fgWatcher = spawn(
  "powershell",
  ["-NoProfile", "-NonInteractive", "-EncodedCommand", Buffer.from(FG_SCRIPT, "utf16le").toString("base64")],
  { stdio: ["ignore", "pipe", "ignore"] },
);
let fgBuf = "";
fgWatcher.stdout.on("data", (d) => {
  fgBuf += d;
  const lines = fgBuf.split(/\r?\n/);
  fgBuf = lines.pop();
  for (const l of lines) if (l.startsWith("FG ")) fgSamples.push(l.trim());
});
/** True when a sample's foreground window belongs to the app under test. */
const fgIsApp = (line) => {
  const rest = line.replace(/^FG \d+ \d+ class=.*? title=\[.*?\] style=0x\w+ ex=0x\w+ ?/, "");
  const [path, cmd = ""] = rest.split("|");
  const low = path.toLowerCase();
  return (
    low === exe.toLowerCase() ||
    low === cliBin.toLowerCase() ||
    (/msedgewebview2/i.test(low) && cmd.includes(`--remote-debugging-port=${port}`))
  );
};

let app;
const problems = [];
// Console noise that is environment, not a bug: Tauri falling back from its IPC custom protocol to postMessage, and the
// updater endpoint answering 404 (no release published for this build) on a runner.
const BENIGN_CONSOLE =
  /IPC custom protocol failed|Failed to fetch|Couldn't find callback id|updater|latest\.json|Failed to load resource: the server responded with a status of 404/i;
// With E2E_VERBOSE the browser writes its own debug log (<profile>/chrome_debug.log) so a missing DevTools port can be explained.
const browserLog = process.env.E2E_VERBOSE ? " --enable-logging --v=0" : "";
const launch = (scale) => {
  // scale: WebView2 --force-device-scale-factor, to run the geometry check at 100/125/150 % without changing the user's display
  app = spawn(exe, [], {
    env: {
      ...env,
      AXELSTRA_TEST_BROWSER_ARGS: `${scale ? `--force-device-scale-factor=${scale}` : ""}${browserLog}`.trim(),
      WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port}${scale ? ` --force-device-scale-factor=${scale}` : ""}${browserLog}`,
    },
    stdio: process.env.E2E_VERBOSE ? "inherit" : "ignore",
  });
  app.on("exit", (c) => (app.gone = c ?? "signal"));
};

/** Version of the installed WebView2 runtime ("none" when the machine has none: the app then opens no page at all). */
function webview2Runtime() {
  const clients = "Microsoft\\EdgeUpdate\\Clients\\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}";
  for (const hive of ["HKLM\\SOFTWARE\\WOW6432Node", "HKLM\\SOFTWARE", "HKCU\\SOFTWARE"]) {
    const r = spawnSync("reg", ["query", `${hive}\\${clients}`, "/v", "pv"], {
      encoding: "utf8",
    });
    const m = r.status === 0 && /pv\s+REG_SZ\s+(\S+)/.exec(r.stdout);
    if (m) return m[1];
  }
  return "none";
}

/** Attaches to the first page target, collecting CSP violations and uncaught exceptions. */
async function connect() {
  let target;
  let seen = "nothing listening on the port";
  for (let i = 0; i < 80 && !target; i++) {
    if (app.gone !== undefined) throw new Error(`app exited early (${app.gone})`);
    try {
      const list = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
      seen = JSON.stringify(list.map((t) => ({ type: t.type, url: t.url })));
      // the page target exists at about:blank before the app navigates; reloading that would leave the window blank
      target = list.find((t) => t.type === "page" && t.webSocketDebuggerUrl && t.url !== "about:blank");
    } catch {
      /* not listening yet */
    }
    if (!target) await sleep(500);
  }
  if (!target)
    throw new Error(
      `no WebView2 page target within 40s (port ${port}); targets: ${seen}; webview2 runtime: ${webview2Runtime()}
${dataDirReport()}`,
    );
  const ws = new WebSocket(target.webSocketDebuggerUrl);
  await new Promise((ok, no) => {
    ws.onopen = ok;
    ws.onerror = () => no(new Error("CDP websocket failed"));
  });
  let seq = 0;
  const pending = new Map();
  ws.onmessage = (m) => {
    const msg = JSON.parse(m.data);
    if (msg.id && pending.has(msg.id)) {
      const { ok, no } = pending.get(msg.id);
      pending.delete(msg.id);
      msg.error ? no(new Error(msg.error.message)) : ok(msg.result);
    } else if (msg.method === "Log.entryAdded") {
      const e = msg.params.entry;
      if (e.level === "error" && !BENIGN_CONSOLE.test(`${e.text} ${e.url ?? ""}`))
        problems.push(`console error: ${e.text} ${e.url ?? ""}`);
    } else if (msg.method === "Runtime.exceptionThrown") {
      problems.push(
        `uncaught: ${msg.params.exceptionDetails.exception?.description ?? msg.params.exceptionDetails.text}`,
      );
    } else if (msg.method === "Audits.issueAdded") {
      const i = msg.params.issue;
      if (i.code === "ContentSecurityPolicyIssue")
        problems.push(`CSP issue: ${JSON.stringify(i.details.contentSecurityPolicyIssueDetails)}`);
    }
  };
  const send = (method, params = {}) =>
    new Promise((ok, no) => {
      const id = ++seq;
      pending.set(id, { ok, no });
      ws.send(JSON.stringify({ id, method, params }));
    });
  for (const d of ["Runtime", "Log", "Audits", "Page"]) await send(`${d}.enable`);
  const evaluate = async (expression) => {
    const r = await send("Runtime.evaluate", {
      expression,
      returnByValue: true,
      awaitPromise: true,
    });
    if (r.exceptionDetails) throw new Error(r.exceptionDetails.exception?.description ?? "evaluate failed");
    return r.result.value;
  };
  // the CSP issues that fired before Audits.enable are replayed by a reload
  await send("Page.reload");
  return { send, evaluate, close: () => ws.close() };
}

async function until(label, fn, ms = 30000) {
  const t0 = Date.now();
  let last;
  while (Date.now() - t0 < ms) {
    try {
      last = await fn();
      if (last) return last;
    } catch (e) {
      last = e;
    }
    await sleep(400);
  }
  throw new Error(`timeout waiting for ${label}${last instanceof Error ? `: ${last.message}` : ""}`);
}

/**
 * Selects the worktree row named "main" (a session opens its terminal once its worktree is selected). The UI can
 * miss a project the daemon only just learned about until its next refresh, so a reload is the retry.
 */
async function selectWorktree(page) {
  const click =
    "(() => { const el = [...document.querySelectorAll('span,div')].find((e) => e.children.length === 0 && e.textContent.trim() === 'main'); const t = el?.closest('[role=button],button,[tabindex]') ?? el; if (!t) return false; t.click(); return true; })()";
  for (let attempt = 0; ; attempt++) {
    try {
      return await until("the worktree row", () => page.evaluate(click), 12000);
    } catch (e) {
      if (attempt >= 3) {
        const seen = await page.evaluate("document.body.innerText.slice(0, 600)").catch((x) => String(x));
        console.error(`sidebar at the failure: ${JSON.stringify(seen)}`);
        throw e;
      }
      await page.send("Page.reload");
    }
  }
}

/** What the throwaway data dir holds, for a failure report (state files and the tail of each log). */
function dataDirReport() {
  const out = [];
  const walk = (d, depth) => {
    for (const e of readdirSync(d, { withFileTypes: true })) {
      const f = join(d, e.name);
      if (e.isDirectory()) {
        if (depth < 2) walk(f, depth + 1);
      } else if (
        /\.(json|log|bad)(\.\d+)?$/.test(e.name) ||
        /^axelstra\.log\./.test(e.name) ||
        e.name === "crash.log"
      ) {
        out.push(`--- ${f}
${readFileSync(f, "utf8").slice(-1500)}`);
      }
    }
  };
  try {
    walk(join(home, "axelstra"), 0);
  } catch (e) {
    out.push(String(e));
  }
  // The host's own log (logs/) and the browser processes tell whether WebView2 was ever started, or died.
  const procs =
    spawnSync("tasklist", ["/FO", "CSV", "/NH"], {
      encoding: "utf8",
      timeout: 20000,
    }).stdout ?? "";
  out.push(`--- processes
${procs
  .split(/\r?\n/)
  .filter((l) => /axelstra|msedge/i.test(l))
  .join("\n")}`);
  // What the browser was started with, and what listens: tells a dropped switch from a port the OS refused.
  const ps = (cmd) =>
    spawnSync("powershell", ["-NoProfile", "-Command", cmd], {
      encoding: "utf8",
      timeout: 30000,
    }).stdout?.trim() ?? "";
  out.push(`--- effective env of the app
${Object.entries(env)
  .filter(([k]) => /^(AXELSTRA_TEST|WEBVIEW2)/.test(k))
  .map(([k, v]) => `${k}=${v}`)
  .join("\n")}`);
  const policyKeys = ["Edge", "Edge\WebView2", "Edge\WebView2\AdditionalBrowserArguments"].flatMap((k) =>
    ["HKLM", "HKCU"].map((h) => `${h}:\SOFTWARE\Policies\Microsoft\${k}`),
  );
  out.push(`--- WebView2/Edge policy keys
${ps(`foreach ($k in ${policyKeys.map((k) => `'${k}'`).join(",")}) { if (Test-Path $k) { "[$k]"; (Get-ItemProperty $k).PSObject.Properties | Where-Object { $_.Name -notlike 'PS*' } | ForEach-Object { "  $($_.Name)=$($_.Value)" } } else { "[$k] absent" } }`)}`);
  out.push(`--- browser command lines
${ps("Get-CimInstance Win32_Process -Filter \"Name='msedgewebview2.exe'\" | Where-Object { $_.CommandLine -notmatch '--type=' } | ForEach-Object { \"$($_.ProcessId) $($_.CommandLine)\" }")}`);
  out.push(`--- listening sockets of the app and browser (wanted port ${port})
${ps('$ids = (Get-Process -Name axelstra-app,msedgewebview2 -ErrorAction SilentlyContinue).Id; Get-NetTCPConnection -State Listen -ErrorAction SilentlyContinue | Where-Object { $ids -contains $_.OwningProcess } | ForEach-Object { "$($_.LocalAddress):$($_.LocalPort) pid $($_.OwningProcess)" }')}`);
  out.push(`--- excluded tcp port ranges
${ps("netsh int ipv4 show excludedportrange protocol=tcp")}`);
  // The profile is <webview>/EBWebView; the browser writes its debug log and DevToolsActivePort inside it.
  for (const name of ["chrome_debug.log", "DevToolsActivePort"]) {
    for (const f of [join(dir, "webview", "EBWebView", name), join(dir, "webview", name)]) {
      try {
        if (existsSync(f))
          out.push(`--- ${f}
${readFileSync(f, "utf8").slice(-3000)}`);
      } catch {
        /* unreadable while the browser holds it */
      }
    }
  }
  try {
    out.push(`--- webview profile: ${readdirSync(join(dir, "webview")).join(" ")}`);
  } catch (e) {
    out.push(`--- webview profile: ${e}`);
  }
  return out.join("\n");
}

let failed = null;
try {
  launch();
  let page = await connect();
  await until("the UI to render", () => page.evaluate("document.getElementById('root')?.children.length > 0")).catch(
    async (e) => {
      const st = await page
        .evaluate(
          "JSON.stringify({url: location.href, ready: document.readyState, html: document.body?.innerHTML.slice(0, 300)})",
        )
        .catch((x) => String(x));
      throw new Error(`${e.message}\npage: ${st}\nproblems: ${[...new Set(problems)].join(" | ")}\n${dataDirReport()}`);
    },
  );
  console.log("ok  boot: the frontend rendered");
  const title = await page.evaluate("document.title");
  if (title !== "Axelstra") throw new Error(`unexpected document title ${JSON.stringify(title)}`);
  console.log("ok  boot: window title");
  // a project with one terminal session, registered through the CLI, must show up in the UI with a live terminal
  await until("the daemon", () => cli("ls").status === 0);
  const repo = join(dir, "repo");
  mkdirSync(repo);
  const git = (...a) => spawnSync("git", a, { cwd: repo, encoding: "utf8" });
  git("init", "-q", "-b", "main");
  git("-c", "user.name=e2e", "-c", "user.email=e2e@example.invalid", "commit", "-q", "--allow-empty", "-m", "init");
  const ap = cli("add-project", repo);
  if (ap.status !== 0) throw new Error(`add-project failed: ${ap.stderr || ap.stdout}`);
  const sp = cli("terminal", repo, "--title", "e2e-term", "--cmd", "echo axelstra-e2e-marker");
  if (sp.status !== 0) {
    const seen = cli("projects");
    throw new Error(`terminal failed: ${sp.stderr || sp.stdout}
projects: ${seen.stdout || seen.stderr}
${dataDirReport()}`);
  }
  console.log("ok  session: project added and terminal started through the CLI");
  // the UI learns about CLI-made changes on its next refresh; a reload is the deterministic way to get one
  await page.send("Page.reload");
  await until("the project in the sidebar", () => page.evaluate("document.body.innerText.includes('repo')"));
  console.log("ok  session: the project is listed");
  await selectWorktree(page);
  await until("the terminal to mount", () => page.evaluate("!!document.querySelector('.xterm')"), 40000);
  console.log("ok  session: an xterm terminal is mounted");
  const id = /\d+/.exec(sp.stdout)?.[0];
  await until("the marker in the session output", () => cli("read", id).stdout.includes("axelstra-e2e-marker"));
  console.log("ok  session: the shell ran the command (axelstra read)");
  await until("the terminal canvas", () =>
    page.evaluate("[...document.querySelectorAll('.xterm canvas')].some((c) => c.width > 0 && c.height > 0)"),
  );
  console.log("ok  session: the terminal has a sized canvas (the WebGL renderer drew)");
  // timeline panel: the "Incluir CI" toggle must reload the timeline without any error toast or uncaught exception
  problems.length = 0;
  await until("the timeline tab", () =>
    page.evaluate(
      "(() => { const t = [...document.querySelectorAll('[role=tab]')].find((e) => e.textContent.trim() === 'Linha do tempo'); if (!t) return false; t.click(); return true; })()",
    ),
  );
  const ciClick =
    "(() => { const b = [...document.querySelectorAll('#rpanel button')].find((e) => e.textContent.trim() === 'Incluir CI'); if (!b) return null; b.click(); return b.getAttribute('aria-pressed'); })()";
  await until("the timeline to load", () => page.evaluate("!!document.querySelector('#rpanel .rlist .pr')"));
  const pressed = await until("the CI toggle", () => page.evaluate(ciClick));
  if (pressed !== "true") throw new Error(`CI toggle aria-pressed is ${pressed} after one click`);
  await sleep(2500);
  const seen = await page.evaluate("document.body.innerText");
  if (/call stack|Maximum/i.test(seen) || (await page.evaluate("!!document.querySelector('#rpanel [role=alert]')")))
    throw new Error(`the CI toggle raised an error: ${seen.slice(0, 600)}`);
  if (problems.length) throw new Error(`CI toggle problems: ${[...new Set(problems)].join(" | ")}`);
  console.log("ok  timeline: the Incluir CI toggle raised no error");
  problems.length = 0; // the page reload above tears xterm down mid-layout; only what happens from here on counts

  // restore: close the window, the daemon keeps the session, a relaunch shows it again
  page.close();
  kill(app.pid);
  await sleep(1500);
  const ls = cli("ls", "--json");
  if (ls.status !== 0 || JSON.parse(ls.stdout).length < 1)
    throw new Error(`daemon lost the session after the app closed: ${ls.stdout}${ls.stderr}`);
  console.log("ok  restore: the daemon kept the session after the app closed");
  launch();
  page = await connect();
  await selectWorktree(page);
  await until("the terminal after relaunch", () => page.evaluate("!!document.querySelector('.xterm')"), 40000);
  console.log("ok  restore: relaunched app shows the session again");
  // addproject: one dialog, six origins. Folders are typed into the fields (the native picker is never opened). The page
  // offers only https://, ssh:// and user@host:path, so a positive clone from a local bare repo (file://) is covered by the
  // daemon tests (crates/daemon/tests/sources.rs); here a failing ssh clone walks the whole job offline.
  problems.length = 0;
  const dlg = (js) => page.evaluate(`(() => { const d = document.querySelector('[role=dialog]'); ${js} })()`);
  const key = async (k, vk) => {
    for (const type of ["keyDown", "keyUp"])
      await page.send("Input.dispatchKeyEvent", {
        type,
        key: k,
        code: k,
        windowsVirtualKeyCode: vk,
      });
  };
  const tabLabel = "(t.querySelector('span')?.textContent ?? '').trim()";
  const tabNames = () => dlg(`return [...d.querySelectorAll('[role=tab]')].map((t) => ${tabLabel});`);
  const selectedTab = () =>
    dlg(`const t = d.querySelector('[role=tab][aria-selected=true]'); return t ? ${tabLabel} : '';`);
  const clickTab = (name) =>
    dlg(
      `const t = [...d.querySelectorAll('[role=tab]')].find((t) => ${tabLabel} === ${JSON.stringify(name)}); if (!t) return false; t.click(); return true;`,
    );
  /** Types the way Solid's onInput sees it: the native value setter plus an input event. */
  const typeInto = (sel, value) =>
    dlg(
      `const i = d.querySelector(${JSON.stringify(sel)}); if (!i) return false; Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set.call(i, ${JSON.stringify(value)}); i.dispatchEvent(new Event('input', { bubbles: true })); return true;`,
    );
  const primary = () =>
    dlg("const b = d.querySelector('[data-ap-primary]'); if (!b || b.disabled) return false; b.click(); return true;");
  const alerts = () =>
    dlg("return [...d.querySelectorAll('[role=alert]')].map((e) => e.textContent.trim()).filter(Boolean).join(' | ');");
  const closed = () => page.evaluate("!document.querySelector('[role=dialog]')");
  const openAdd = async () => {
    await until("the add button", () =>
      page.evaluate(
        "(() => { const b = [...document.querySelectorAll('button')].find((e) => /^[+] (Adicionar projeto|Add project)$/.test(e.innerText.trim())); if (!b) return false; b.click(); return true; })()",
      ),
    );
    await until("the dialog", () => page.evaluate("!!document.querySelector('[role=dialog]')"));
  };

  await openAdd();
  const want = ["Pasta local", "Clonar repositório", "GitHub", "Novo projeto", "Remoto", "Descobertos"];
  const names = await tabNames();
  if (JSON.stringify(names) !== JSON.stringify(want)) throw new Error(`add project tabs are ${JSON.stringify(names)}`);
  console.log("ok  addproject: the dialog lists the six origins");

  // arrows move AND select, Home and End jump
  await dlg("d.querySelector('[role=tab][aria-selected=true]').focus();");
  await key("Home", 36);
  await until("Home", async () => (await selectedTab()) === want[0], 3000);
  for (const next of want.slice(1)) {
    await key("ArrowDown", 40);
    await until(`arrow down to ${next}`, async () => (await selectedTab()) === next, 3000);
  }
  await key("Home", 36);
  await key("End", 35);
  await until("End", async () => (await selectedTab()) === want[5], 3000);
  await key("ArrowUp", 38);
  await until("arrow up", async () => (await selectedTab()) === want[4], 3000);
  console.log("ok  addproject: arrows, Home and End move and select the origin");

  // Novo projeto: a bad name is refused in the page; a good one makes folder + README + first commit and adds it
  const parent = join(dir, "newparent");
  mkdirSync(parent);
  await clickTab("Novo projeto");
  await until("the new form", () => page.evaluate("!!document.querySelector('#ap-new-parent')"));
  await typeInto("#ap-new-parent", parent);
  await typeInto("input[placeholder=o-meu-projeto]", "a/b");
  await primary();
  await until("the name problem", async () => /uma só pasta/.test(await alerts()), 5000);
  console.log("ok  addproject: new project: a name with a separator is refused before the daemon");
  await typeInto("input[placeholder=o-meu-projeto]", "e2e-new");
  await primary();
  await until("the dialog to close", closed, 30000);
  const made = join(parent, "e2e-new");
  if (!existsSync(join(made, "README.md"))) throw new Error("project.init wrote no README.md");
  const commits = spawnSync("git", ["rev-list", "--count", "HEAD"], {
    cwd: made,
    env,
    encoding: "utf8",
  });
  if (commits.stdout.trim() !== "1") throw new Error(`expected one commit, got ${commits.stdout}${commits.stderr}`);
  await until("the project to be registered", () => cli("projects").stdout.includes("e2e-new"));
  console.log("ok  addproject: new project: folder, README, first commit and registration");

  // the dialog opens on the origin used last
  await openAdd();
  if ((await selectedTab()) !== "Novo projeto")
    throw new Error(`the dialog did not remember its last origin: ${await selectedTab()}`);
  console.log("ok  addproject: the dialog reopens on the last origin used");

  // Clonar: file:// is refused by the page; an unreachable ssh address fails offline and leaves no folder behind
  await clickTab("Clonar repositório");
  await until("the clone form", () => page.evaluate("!!document.querySelector('#ap-clone-parent')"));
  await typeInto("#ap-clone-parent", parent);
  await typeInto("input[placeholder^='https://github.com']", "file:///tmp/none.git");
  await primary();
  await until("the scheme problem", async () => /https:\/\//.test(await alerts()), 5000);
  console.log("ok  addproject: clone: file:// is refused by the page");
  await typeInto("input[placeholder^='https://github.com']", "ssh://127.0.0.1:1/none.git");
  await primary();
  await until("the clone to fail", async () => /A clonagem falhou/.test(await alerts()), 60000);
  if (existsSync(join(parent, "none"))) throw new Error("a failed clone left its folder behind");
  console.log("ok  addproject: clone: a failing clone reports the error and removes the partial folder");

  // GitHub: no gh login in this profile, so the tab explains it instead of listing
  await clickTab("GitHub");
  await until("the gh hint", async () => /gh/.test(await alerts()), 30000);
  if (await dlg("return !!d.querySelector('#ap-gh-list');"))
    throw new Error("the GitHub tab listed repositories without a login");
  console.log("ok  addproject: GitHub: no gh login shows its hint and no list");

  // Descobertos scans the user's real home read-only (USERPROFILE cannot be overridden); either result is fine and
  // nothing is added from it. Adding a discovered repository is covered by the daemon tests.
  await clickTab("Descobertos");
  await until("the discovered list or its empty note", () =>
    dlg("return !!d.querySelector('#ap-ds-list') || /Não encontrei/.test(d.innerText);"),
  );
  console.log("ok  addproject: Descobertos: lists repositories or says there are none");

  await key("Escape", 27);
  await until("Esc to close the dialog", closed, 5000);
  console.log("ok  addproject: Esc closes the dialog");
  await sleep(500);
  if (problems.length) throw new Error(`add project problems: ${[...new Set(problems)].join(" | ")}`);
  console.log("ok  addproject: no console error, CSP violation or uncaught exception");
  problems.length = 0;

  // removeproject: the sidebar X asks how to remove the project. Temp repo with two worktrees (one dirty, one with an
  // unintegrated commit), never a real project. "list" keeps every folder; "all" needs the typed name and deletes them.
  problems.length = 0;
  const rmRepo = join(dir, "rmrepo");
  // the daemon only previews and removes worktrees under `<repo parent>/.axelstra-wt` (the only door to deleting a folder),
  // and only counts an `ax/*` branch as unintegrated work
  const rmWts = join(dir, ".axelstra-wt");
  mkdirSync(rmRepo);
  mkdirSync(rmWts);
  const rg = (cwd, ...a) => {
    const r = spawnSync("git", a, { cwd, env, encoding: "utf8" });
    if (r.status !== 0) throw new Error(`git ${a.join(" ")} failed: ${r.stderr}`);
  };
  rg(rmRepo, "init", "-b", "main");
  writeFileSync(join(rmRepo, "README.md"), "rm\n");
  rg(rmRepo, "add", "README.md");
  rg(rmRepo, "commit", "-m", "init");
  const wtDirty = join(rmWts, "rmrepo-dirty");
  const wtUnmerged = join(rmWts, "rmrepo-unmerged");
  rg(rmRepo, "worktree", "add", "-b", "ax/e2e-dirty", wtDirty);
  writeFileSync(join(wtDirty, "README.md"), "changed and not committed\n");
  rg(rmRepo, "worktree", "add", "-b", "ax/e2e-unmerged", wtUnmerged);
  writeFileSync(join(wtUnmerged, "work.txt"), "work\n");
  rg(wtUnmerged, "add", "work.txt");
  rg(wtUnmerged, "commit", "-m", "unintegrated work");
  const addRm = async () => {
    const r = cli("add-project", rmRepo);
    if (r.status !== 0) throw new Error(`add-project failed: ${r.stderr || r.stdout}`);
    await until("rmrepo in the sidebar", () =>
      page.evaluate("[...document.querySelectorAll('.pname')].some((e) => e.textContent.trim() === 'rmrepo')"),
    );
  };
  const openRemove = async () => {
    await until("the remove button of rmrepo", () =>
      page.evaluate(
        "(() => { const n = [...document.querySelectorAll('.pname')].find((e) => e.textContent.trim() === 'rmrepo'); const b = n?.parentElement?.querySelector('button[aria-label=\"Remover projeto\"]'); if (!b) return false; b.click(); return true; })()",
      ),
    );
    await until("the remove dialog", () =>
      page.evaluate("!!document.querySelector('[role=dialog] .rm-risk, [role=dialog] input[type=radio]')"),
    );
    // "Remover tudo" can only be chosen once the preview is in (the page ignores the click before)
    await until("the preview to load", async () => /e2e-dirty/.test(await dlg("return d.innerText;")), 15000);
  };
  const radioOf = (v) =>
    dlg(
      `const r = d.querySelector('input[type=radio][value="${v}"]'); if (!r) return false; (r.labels?.[0] ?? r).click(); return true;`,
    );
  const checkedMode = () => dlg("return d.querySelector('input[type=radio]:checked')?.value ?? '';");
  const okState = () =>
    dlg("const b = d.querySelector('.m-actions .primary'); return b ? (b.disabled ? 'disabled' : 'enabled') : 'none';");
  const pressOk = () =>
    dlg(
      "const b = d.querySelector('.m-actions .primary'); if (!b || b.disabled) return false; b.click(); return true;",
    );
  const projectListed = () => /"name": "rmrepo"/.test(cli("projects").stdout);

  await addRm();
  await openRemove();
  if ((await checkedMode()) !== "list")
    throw new Error(`the remove dialog did not default to list: ${await checkedMode()}`);
  await until("the preview to load", async () => /e2e-dirty/.test(await dlg("return d.innerText;")), 15000);
  const text = await dlg("return d.innerText;");
  if (!/e2e-unmerged/.test(text)) throw new Error(`the unintegrated branch is not warned about: ${text}`);
  const red = await dlg("const e = d.querySelector('.rm-risk'); return e ? getComputedStyle(e).color : '';");
  if (!red) throw new Error("the risk box is missing");
  console.log("ok  removeproject: defaults to list and warns about the dirty worktree and the unintegrated branch");

  // cancel removes nothing
  await dlg("[...d.querySelectorAll('.m-actions button')][0].click();");
  await until("the dialog to close", closed, 5000);
  if (!projectListed()) throw new Error("cancel removed the project");
  console.log("ok  removeproject: cancel keeps the project");

  // "all" with unsaved work stays disabled until the project name is typed
  await openRemove();
  await radioOf("all");
  await until("all selected", async () => (await checkedMode()) === "all", 3000);
  await until("the typed-name guard", async () => (await okState()) === "disabled", 3000);
  await typeInto("input[placeholder=rmrepo]", "rmrepo");
  await until("the OK button to enable", async () => (await okState()) === "enabled", 3000);
  console.log("ok  removeproject: 'all' with unsaved work needs the typed name");

  // back to "list": confirm; the project leaves the list, every folder stays
  await radioOf("list");
  await until("list selected", async () => (await checkedMode()) === "list", 3000);
  await pressOk();
  await until("the dialog to close", closed, 5000);
  await until("the project to leave the list", () => !projectListed(), 15000);
  if (!existsSync(wtDirty) || !existsSync(wtUnmerged) || !existsSync(rmRepo))
    throw new Error("'list' deleted a folder");
  console.log("ok  removeproject: 'list' drops the project and deletes nothing");

  // "all": worktree folders and the project go; the main repository stays
  await addRm();
  await openRemove();
  await radioOf("all");
  await until("all selected", async () => (await checkedMode()) === "all", 3000);
  await typeInto("input[placeholder=rmrepo]", "rmrepo");
  await until("the OK button to enable", async () => (await okState()) === "enabled", 3000);
  await pressOk();
  await until("the dialog to close", closed, 5000);
  await until("the project to leave the list", () => !projectListed(), 30000);
  await until("the worktree folders to go", () => !existsSync(wtDirty) && !existsSync(wtUnmerged), 30000);
  if (!existsSync(join(rmRepo, "README.md"))) throw new Error("'all' touched the main repository");
  await until(
    "the notice with the counts",
    () => page.evaluate("/Projeto removido: rmrepo/.test(document.body.innerText)"),
    10000,
  );
  console.log("ok  removeproject: 'all' removes worktrees and project, keeps the main repository, shows the counts");
  await sleep(500);
  if (problems.length) throw new Error(`remove project problems: ${[...new Set(problems)].join(" | ")}`);
  console.log("ok  removeproject: no console error, CSP violation or uncaught exception");
  problems.length = 0;

  // the app must never have been the foreground window (whoever else changed it, e.g. the runner, is not our doing)
  const stolen = fgSamples.filter(fgIsApp);
  if (stolen.length)
    throw new Error(`an app window became the foreground window (it stole the focus): ${stolen.join(" ; ")}`);
  if (fgSamples.length === 0) throw new Error("the foreground watcher produced no sample: the check would be blind");
  console.log(`ok  no app window was ever the foreground window (${fgSamples.length} foreground samples)`);
  // no white band: #root fills the viewport and the corners paint the theme, not the default white
  const fill = await page.evaluate(
    "(() => { const r = document.getElementById('root').getBoundingClientRect(); return [r.left, r.top, r.width - innerWidth, r.height - innerHeight].map(Math.round); })()",
  );
  if (fill.some((v) => Math.abs(v) > 1)) throw new Error(`#root does not fill the window: ${fill}`);
  const bg = await page.evaluate("getComputedStyle(document.documentElement).backgroundColor");
  if (/^rgb\(255, 255, 255\)|^rgba\(0, 0, 0, 0\)/.test(bg)) throw new Error(`html background is ${bg}`);
  console.log("ok  #root fills the window, html background is the theme");

  // geometry: the viewport is the window's client area and nothing is displaced, at 100/125/150 % and after real resizes
  const geometry = async (label) => {
    const info = windowInfo(app.pid);
    const v = JSON.parse(
      await page.evaluate(
        "(() => { const r = document.getElementById('root').getBoundingClientRect(); const b = document.documentElement.getBoundingClientRect(); return JSON.stringify({ iw: innerWidth, ih: innerHeight, dpr: devicePixelRatio, root: [r.left, r.top, r.width, r.height], html: [b.left, b.top], vv: [visualViewport.offsetLeft, visualViewport.offsetTop], sx: scrollX, sy: scrollY }); })()",
      ),
    );
    const bad = layoutProblems(info);
    if (Math.abs(v.iw * v.dpr - info.clientW) > 1.5 || Math.abs(v.ih * v.dpr - info.clientH) > 1.5)
      bad.push(`viewport ${v.iw}x${v.ih} @${v.dpr} is not the client area ${info.clientW}x${info.clientH}`);
    if ([...v.root.slice(0, 2), ...v.html, ...v.vv, v.sx, v.sy].some((n) => Math.abs(n) > 0.5))
      bad.push(
        `layout offset: root ${v.root.slice(0, 2)}, html ${v.html}, visualViewport ${v.vv}, scroll ${v.sx},${v.sy}`,
      );
    if (Math.abs(v.root[2] - v.iw) > 1 || Math.abs(v.root[3] - v.ih) > 1)
      bad.push(`#root ${v.root[2]}x${v.root[3]} does not fill ${v.iw}x${v.ih}`);
    if (bad.length) throw new Error(`geometry (${label}): ${bad.join("; ")}`);
    console.log(
      `ok  geometry ${label}: viewport ${v.iw}x${v.ih} @${v.dpr} = client ${info.clientW}x${info.clientH}, no offset`,
    );
  };
  await geometry("100%");
  for (const [w, h] of [
    [1000, 700],
    [900, 600],
  ]) {
    resizeClient(app.pid, w, h);
    await sleep(700);
    await geometry(`100% resized to ${w}x${h}`);
  }
  for (const scale of ["1.25", "1.5"]) {
    page.close();
    kill(app.pid, true);
    await sleep(1500);
    launch(scale);
    page = await connect();
    await until("the UI at scale " + scale, () =>
      page.evaluate("document.getElementById('root')?.children.length > 0"),
    );
    await sleep(1000);
    await geometry(`${Number(scale) * 100}%`);
  }

  await sleep(1000);
  if (problems.length) throw new Error(`browser problems:\n  ${[...new Set(problems)].join("\n  ")}`);
  console.log("ok  no CSP violation, console error or uncaught exception");
  // the policy must really be enforced: a <style> that did not get the page's nonce has to be refused
  const blocked = await page.evaluate(
    "(() => { document.head.insertAdjacentHTML('beforeend', '<style>body{outline:3px solid red}</style>'); return getComputedStyle(document.body).outlineStyle === 'none'; })()",
  );
  if (!blocked) throw new Error("CSP is not enforcing: an un-nonced <style> was applied");
  console.log("ok  CSP enforced: an injected <style> without the nonce is refused");
  page.close();
} catch (e) {
  failed = e;
  console.error(`FAIL e2e: ${e.stack ?? e}`);
  if (problems.length) console.error(`browser problems so far:\n  ${[...new Set(problems)].join("\n  ")}`);
} finally {
  fgWatcher.kill();
  if (app) kill(app.pid, true);
  // the daemon is detached from the app: stop the ones started from this build tree only (never the developer's own)
  spawnSync(
    "powershell",
    [
      "-NoProfile",
      "-Command",
      `Get-CimInstance Win32_Process | Where-Object { $_.ExecutablePath -in '${cliBin}','${exe}' } | ForEach-Object { Stop-Process -Id $_.ProcessId -Force }`,
    ],
    { stdio: "ignore" },
  );
  await sleep(500);
  try {
    rmSync(dir, { recursive: true, force: true });
  } catch {
    /* a lingering handle: the OS temp cleanup gets it */
  }
}
if (failed) process.exit(1);
console.log("e2e: all scenarios passed");
