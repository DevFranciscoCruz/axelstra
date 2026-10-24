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
// shows up in the UI with a live terminal), restore (the app is closed and relaunched: the daemon kept the session).
import { spawn, spawnSync } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

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

const dir = mkdtempSync(join(tmpdir(), "axelstra-e2e-"));
const home = join(dir, "home");
mkdirSync(home, { recursive: true });
const port = 9300 + (process.pid % 500);
const user = `axe2e${process.pid}`;
// USERPROFILE stays real: overriding it makes WebView2 exit at startup. LOCALAPPDATA is what proto::data_dir() reads.
const env = {
  ...process.env,
  HOME: home,
  LOCALAPPDATA: home,
  XDG_DATA_HOME: join(home, "share"),
  USERNAME: user,
  USER: user,
  WEBVIEW2_USER_DATA_FOLDER: join(dir, "webview"),
  WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port}`,
};
const cli = (...a) => spawnSync(cliBin, a, { env, encoding: "utf8", timeout: 20000 });
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
// without /T the daemon (a child of the app) survives, as it does when the user closes the window
const kill = (pid, tree = false) =>
  spawnSync("taskkill", ["/PID", String(pid), ...(tree ? ["/T"] : []), "/F"], { stdio: "ignore" });

let app;
const problems = [];
const launch = () => {
  app = spawn(exe, [], { env, stdio: process.env.E2E_VERBOSE ? "inherit" : "ignore" });
  app.on("exit", (c) => (app.gone = c ?? "signal"));
};

/** Attaches to the first page target, collecting CSP violations and uncaught exceptions. */
async function connect() {
  let target;
  for (let i = 0; i < 80 && !target; i++) {
    if (app.gone !== undefined) throw new Error(`app exited early (${app.gone})`);
    try {
      const list = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
      target = list.find((t) => t.type === "page" && t.webSocketDebuggerUrl);
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
    } else if (msg.method === "Log.entryAdded") {
      const e = msg.params.entry;
      if (e.level === "error") problems.push(`console error: ${e.text} ${e.url ?? ""}`);
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
    const r = await send("Runtime.evaluate", { expression, returnByValue: true, awaitPromise: true });
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
      if (attempt >= 3) throw e;
      await page.send("Page.reload");
    }
  }
}

let failed = null;
try {
  launch();
  let page = await connect();
  await until("the UI to render", () => page.evaluate("document.getElementById('root')?.children.length > 0"));
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
  if (sp.status !== 0) throw new Error(`terminal failed: ${sp.stderr || sp.stdout}`);
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
