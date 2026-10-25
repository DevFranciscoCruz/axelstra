// Hidden E2E of "Ctrl+click an .html path in the terminal opens it in the browser" (Windows, WebView2, CDP).
//
//   cargo build -p axelstra && (cd app && npm run build) && cargo build -p axelstra-app --features tauri/custom-protocol
//   node tools/e2e-html-link.mjs [path\to\axelstra-app.exe]
//
// Same isolation as tools/e2e.mjs (docs/E2E.md): AXELSTRA_TEST_HIDDEN=1, a temp AXELSTRA_DATA_DIR, a test pipe, a private
// WebView2 profile. No browser is opened: AXELSTRA_TEST_OPENER_LOG=<file> makes the host append the resolved file:// url
// to that file instead of calling the system opener. A session prints three paths; a real click (CDP input events,
// with the Control modifier where noted) on each is checked against the log:
//   page.html            inside the session cwd     -> Ctrl+click logs its file:// url; a plain click logs nothing
//   notes.txt            not an html file           -> no link, nothing logged
//   ..\outside\x.html    escapes the cwd            -> the daemon refuses it, nothing logged
import { spawn, spawnSync } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

if (process.platform !== "win32") {
  console.log("e2e-html-link: WebView2 driver is Windows only; skipping");
  process.exit(0);
}
const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const exe = resolve(process.argv[2] ?? process.env.AXELSTRA_APP ?? join(root, "target", "debug", "axelstra-app.exe"));
const cliBin = join(dirname(exe), "axelstra.exe");
for (const f of [exe, cliBin]) {
  if (!existsSync(f)) {
    console.error(`e2e-html-link: missing ${f} (see the header of tools/e2e-html-link.mjs)`);
    process.exit(2);
  }
}

const dir = mkdtempSync(join(tmpdir(), "axelstra-e2e-html-"));
const home = join(dir, "home");
mkdirSync(home, { recursive: true });
const port = 9800 + (process.pid % 500);
const user = `axehtml${process.pid}`;
const openerLog = join(dir, "opener.log");
const env = {
  ...process.env,
  HOME: home,
  LOCALAPPDATA: home, // USERPROFILE stays real: overriding it makes WebView2 exit at startup
  AXELSTRA_DATA_DIR: join(home, "axelstra"),
  XDG_DATA_HOME: join(home, "share"),
  USERNAME: user,
  USER: user,
  AXELSTRA_TEST_PIPE: user,
  AXELSTRA_TEST_HIDDEN: process.env.AXELSTRA_TEST_HIDDEN ?? "1",
  AXELSTRA_TEST_OPENER_LOG: openerLog,
  WEBVIEW2_USER_DATA_FOLDER: join(dir, "webview"),
  WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port}`,
};
const cli = (...a) => spawnSync(cliBin, a, { env, encoding: "utf8", timeout: 20000 });
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const kill = (pid) => spawnSync("taskkill", ["/PID", String(pid), "/T", "/F"], { stdio: "ignore" });
const logLines = () => (existsSync(openerLog) ? readFileSync(openerLog, "utf8").split(/\r?\n/).filter(Boolean) : []);

let app;
async function connect() {
  let target;
  for (let i = 0; i < 80 && !target; i++) {
    if (app.gone !== undefined) throw new Error(`app exited early (${app.gone})`);
    try {
      const list = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
      target = list.find((t) => t.type === "page" && t.webSocketDebuggerUrl && t.url !== "about:blank");
    } catch {
      /* not listening yet */
    }
    if (!target) await sleep(500);
  }
  if (!target) throw new Error(`no WebView2 page target within 40s (port ${port})`);
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
    }
  };
  const send = (method, params = {}) =>
    new Promise((ok, no) => {
      const id = ++seq;
      pending.set(id, { ok, no });
      ws.send(JSON.stringify({ id, method, params }));
    });
  const evaluate = async (expression) => {
    const r = await send("Runtime.evaluate", {
      expression,
      returnByValue: true,
      awaitPromise: true,
    });
    if (r.exceptionDetails) throw new Error(r.exceptionDetails.exception?.description ?? "evaluate failed");
    return r.result.value;
  };
  await send("Runtime.enable");
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

const CTRL = 2; // CDP Input modifiers bitmask: Alt 1, Ctrl 2, Meta 4, Shift 8

/** moves over, then clicks (optionally with Ctrl) the cell at (col, row) of the terminal screen */
async function clickCell(page, col, row, modifiers) {
  const g = await page.evaluate(`(() => {
    const s = document.querySelector('.xterm-screen').getBoundingClientRect();
    const m = document.querySelector('.xterm-char-measure-element').getBoundingClientRect();
    return { left: s.left, top: s.top, cw: m.width / 32, ch: m.height };
  })()`);
  const x = g.left + g.cw * (col + 0.5);
  const y = g.top + g.ch * (row + 0.5);
  const base = { x, y, modifiers };
  await page.send("Input.dispatchMouseEvent", { ...base, type: "mouseMoved" });
  await sleep(250); // xterm asks the link providers on the move
  await page.send("Input.dispatchMouseEvent", {
    ...base,
    type: "mousePressed",
    button: "left",
    clickCount: 1,
  });
  await page.send("Input.dispatchMouseEvent", {
    ...base,
    type: "mouseReleased",
    button: "left",
    clickCount: 1,
  });
  await sleep(400);
}

/**
 * Clicks the middle of `needle` on every terminal row (the buffer is not reachable from here) until `done()` holds;
 * returns whether it did.
 */
async function clickText(page, needle, modifiers, done) {
  const text = cli("read", session).stdout.split(/\r?\n/);
  const col = text.map((l) => l.indexOf(needle)).find((c) => c >= 0);
  if (col === undefined) throw new Error(`"${needle}" is not in the terminal output:\n${text.join("\n")}`);
  const rows = await page.evaluate(
    "Math.floor(document.querySelector('.xterm-screen').getBoundingClientRect().height / document.querySelector('.xterm-char-measure-element').getBoundingClientRect().height)",
  );
  for (let r = 0; r < rows; r++) {
    await clickCell(page, col + Math.floor(needle.length / 2), r, modifiers);
    if (done()) return true;
  }
  return false;
}

let session;
let failed = null;
try {
  app = spawn(exe, [], {
    env,
    stdio: process.env.E2E_VERBOSE ? "inherit" : "ignore",
  });
  app.on("exit", (c) => (app.gone = c ?? "signal"));
  const page = await connect();
  await until("the UI to render", () => page.evaluate("document.getElementById('root')?.children.length > 0"));
  await until("the daemon", () => cli("ls").status === 0);

  // a git project whose cwd holds page.html and notes.txt; outside/x.html exists but is outside the cwd
  const repo = join(dir, "repo");
  mkdirSync(repo);
  const git = (...a) => spawnSync("git", a, { cwd: repo, encoding: "utf8" });
  git("init", "-q", "-b", "main");
  git("-c", "user.name=e2e", "-c", "user.email=e2e@example.invalid", "commit", "-q", "--allow-empty", "-m", "init");
  mkdirSync(join(dir, "outside"));
  writeFileSync(join(dir, "outside", "x.html"), "<p>outside</p>");
  writeFileSync(join(repo, "page.html"), "<p>inside</p>");
  writeFileSync(join(repo, "notes.txt"), "not html");
  writeFileSync(
    join(repo, "print.js"),
    [
      'console.log("INSIDE page.html");',
      'console.log("TEXT notes.txt");',
      'console.log("OUTSIDE ..\\\\outside\\\\x.html");',
      "setInterval(() => {}, 1e6);",
    ].join("\n"),
  );
  if (cli("add-project", repo).status !== 0) throw new Error("add-project failed");
  const sp = cli("terminal", repo, "--title", "e2e-html", "--cmd", "node print.js");
  if (sp.status !== 0) throw new Error(`terminal failed: ${sp.stderr || sp.stdout}`);
  session = /\d+/.exec(sp.stdout)?.[0];
  await until("the output", () => cli("read", session).stdout.includes("OUTSIDE"));
  await page.send("Page.reload");
  await until("the project in the sidebar", () => page.evaluate("document.body.innerText.includes('repo')"));
  const pick =
    "(() => { const el = [...document.querySelectorAll('span,div')].find((e) => e.children.length === 0 && e.textContent.trim() === 'main'); const t = el?.closest('[role=button],button,[tabindex]') ?? el; if (!t) return false; t.click(); return true; })()";
  await until("the worktree row", () => page.evaluate(pick), 20000);
  await until("the terminal to mount", () => page.evaluate("!!document.querySelector('.xterm-screen')"), 40000);
  console.log("ok  terminal mounted with the printed paths");

  // 1. a plain click on the html path logs nothing
  if (await clickText(page, "page.html", 0, () => logLines().length > 0))
    throw new Error("a click without Ctrl opened a file");
  console.log("ok  plain click does not open");

  // 2. Ctrl+click on the html path inside the cwd logs its file:// url
  const want = pathToFileURL(join(repo, "page.html")).href.toLowerCase();
  if (!(await clickText(page, "page.html", CTRL, () => logLines().length > 0)))
    throw new Error("Ctrl+click on page.html logged nothing");
  const got = logLines();
  if (got.length !== 1 || got[0].toLowerCase() !== want)
    throw new Error(`opener log is ${JSON.stringify(got)}, wanted ${want}`);
  console.log(`ok  Ctrl+click on page.html opened ${got[0]}`);

  // 3. a non-html path and an html path outside the cwd must not open anything
  const before = logLines().length;
  await clickText(page, "notes.txt", CTRL, () => false);
  console.log("ok  Ctrl+click on notes.txt did not open");
  await clickText(page, "x.html", CTRL, () => false);
  if (logLines().length !== before) throw new Error(`a refused path was opened: ${JSON.stringify(logLines())}`);
  console.log("ok  Ctrl+click on ..\\outside\\x.html did not open");
  page.close();
} catch (e) {
  failed = e;
  console.error(`FAIL e2e-html-link: ${e.stack ?? e}`);
} finally {
  if (app) kill(app.pid);
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
console.log("e2e-html-link: all scenarios passed");
