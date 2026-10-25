// Hidden E2E of the Kanban drag and drop (Windows, WebView2, CDP).
//
//   cargo build -p axelstra && (cd app && npm run build) && cargo build -p axelstra-app --features tauri/custom-protocol
//   node tools/e2e-kanban.mjs [path\to\axelstra-app.exe]
//
// Same isolation as tools/e2e.mjs (docs/E2E.md): AXELSTRA_TEST_HIDDEN=1, a temp AXELSTRA_DATA_DIR, a test pipe, a private
// WebView2 profile. Cards are .kanban/*.md files in a throwaway git repo; real mouse and key events (CDP Input) drive
// the board and the card files are read back:
//   pointer drag to another column, reorder inside a column (kept after a reload), Esc cancels, dropping outside the
//   columns cancels, keyboard (Space, ArrowRight, Space) moves a card, announces it and keeps focus, and the old
//   "Mover para" button is gone.
import { spawn, spawnSync } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

if (process.platform !== "win32") {
  console.log("e2e-kanban: WebView2 driver is Windows only; skipping");
  process.exit(0);
}
const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const exe = resolve(process.argv[2] ?? process.env.AXELSTRA_APP ?? join(root, "target", "debug", "axelstra-app.exe"));
const cliBin = join(dirname(exe), "axelstra.exe");
for (const f of [exe, cliBin]) {
  if (!existsSync(f)) {
    console.error(`e2e-kanban: missing ${f} (see the header of tools/e2e-kanban.mjs)`);
    process.exit(2);
  }
}

const dir = mkdtempSync(join(tmpdir(), "axelstra-e2e-kanban-"));
const home = join(dir, "home");
mkdirSync(home, { recursive: true });
const port = 9300 + (process.pid % 500);
const user = `axekb${process.pid}`;
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
  WEBVIEW2_USER_DATA_FOLDER: join(dir, "webview"),
  WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port}`,
};
const cli = (...a) => spawnSync(cliBin, a, { env, encoding: "utf8", timeout: 20000 });
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const kill = (pid) => spawnSync("taskkill", ["/PID", String(pid), "/T", "/F"], { stdio: "ignore" });

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

const key = async (page, k, code, vk, text) => {
  await page.send("Input.dispatchKeyEvent", {
    type: text ? "keyDown" : "rawKeyDown",
    key: k,
    code,
    windowsVirtualKeyCode: vk,
    ...(text ? { text } : {}),
  });
  await page.send("Input.dispatchKeyEvent", {
    type: "keyUp",
    key: k,
    code,
    windowsVirtualKeyCode: vk,
  });
  await sleep(200);
};
const mouse = (page, type, x, y) =>
  page.send("Input.dispatchMouseEvent", {
    type,
    x,
    y,
    button: "left",
    buttons: type === "mouseReleased" ? 0 : 1,
    clickCount: 1,
  });
/** a point inside an element: `frac` is the vertical position (0 top, 1 bottom) */
const where = (page, sel, frac = 0.5) =>
  page.evaluate(
    `(() => { const r = document.querySelector(${JSON.stringify(sel)}).getBoundingClientRect(); return { x: r.left + r.width / 2, y: r.top + r.height * ${frac} }; })()`,
  );
const card = (id) => `.kcard[data-id="${id}"]`;
const order = (page, n) =>
  page.evaluate(`[...document.querySelectorAll('.kcol:nth-child(${n}) .kcard')].map((e) => e.dataset.id).join(',')`);
async function drag(page, from, to, { esc = false } = {}) {
  await mouse(page, "mouseMoved", from.x, from.y);
  await mouse(page, "mousePressed", from.x, from.y);
  for (let i = 1; i <= 8; i++)
    await mouse(page, "mouseMoved", from.x + ((to.x - from.x) * i) / 8, from.y + ((to.y - from.y) * i) / 8);
  await sleep(200);
  if (!(await page.evaluate("!!document.querySelector('.kghost')"))) throw new Error("no ghost card while dragging");
  if (esc) await key(page, "Escape", "Escape", 27);
  await mouse(page, "mouseReleased", to.x, to.y);
  await sleep(1200); // the commit writes the files, then reloads
}

const repo = join(dir, "repo");
const file = (id) => readFileSync(join(repo, ".kanban", `${id}.md`), "utf8");
const status = (id) => /^status: (\S+)/m.exec(file(id))?.[1];
const prio = (id) => Number(/^priority: (-?\d+)/m.exec(file(id))?.[1] ?? 0);
const expect = (label, got, want) => {
  if (JSON.stringify(got) !== JSON.stringify(want))
    throw new Error(`${label}: got ${JSON.stringify(got)}, wanted ${JSON.stringify(want)}`);
  console.log(`ok  ${label}`);
};

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

  mkdirSync(repo);
  const git = (...a) => spawnSync("git", a, { cwd: repo, encoding: "utf8" });
  git("init", "-q", "-b", "main");
  git("-c", "user.name=e2e", "-c", "user.email=e2e@example.invalid", "commit", "-q", "--allow-empty", "-m", "init");
  mkdirSync(join(repo, ".kanban"));
  for (const [id, st] of [
    ["a", "todo"],
    ["b", "todo"],
    ["c", "done"],
    ["d", "done"],
  ])
    writeFileSync(
      join(repo, ".kanban", `${id}.md`),
      `---\nid: ${id}\ntitle: Card ${id}\nstatus: ${st}\n---\n\nbody ${id}\n`,
    );
  if (cli("add-project", repo).status !== 0) throw new Error("add-project failed");
  await page.send("Page.reload");
  await until("the project in the sidebar", () => page.evaluate("document.body.innerText.includes('repo')"));
  const click = (js) =>
    page.evaluate(`(() => { const el = ${js}; if (!el) return false; el.click(); return true; })()`);
  const openBoard = async () => {
    await until("the project selected", () =>
      click(
        "[...document.querySelectorAll('span,div')].find((e) => e.children.length === 0 && e.textContent.trim() === 'repo')",
      ),
    );
    await until("the main worktree", () =>
      click(
        "(() => { const el = [...document.querySelectorAll('span,div')].find((e) => e.children.length === 0 && e.textContent.trim() === 'main'); return el?.closest('[role=button],button,[tabindex]') ?? el; })()",
      ),
    );
    await until("the Kanban tab", () =>
      click("[...document.querySelectorAll('[role=tab],button')].find((e) => /^kanban$/i.test(e.textContent.trim()))"),
    );
  };
  await openBoard();
  await until("the cards", () => page.evaluate("document.querySelectorAll('.kcard').length === 4"));
  expect("the board lists the cards", [await order(page, 2), await order(page, 5)], ["a,b", "c,d"]);
  expect(
    "no 'Mover para' button is left",
    await page.evaluate(
      "[...document.querySelectorAll('.kcard button')].some((b) => /mover|move to/i.test(b.textContent))",
    ),
    false,
  );

  // 1. pointer: a -> top of the done column
  await drag(page, await where(page, card("a")), await where(page, card("c"), 0.2));
  expect("pointer drag moves a to done, above c", [await order(page, 2), await order(page, 5)], ["b", "a,c,d"]);
  expect("the card file says done", status("a"), "done");

  // 2. reorder inside a column: d to the top of done
  await drag(page, await where(page, card("d")), await where(page, card("a"), 0.1));
  expect("reorder inside a column", await order(page, 5), "d,a,c");
  if (!(prio("d") > prio("a"))) throw new Error(`d must outrank a (priority ${prio("d")} vs ${prio("a")})`);
  console.log("ok  the order is kept in the cards' priority");
  await page.send("Page.reload");
  await until("the project in the sidebar again", () => page.evaluate("document.body.innerText.includes('repo')"));
  await openBoard();
  await until("the cards again", () => page.evaluate("document.querySelectorAll('.kcard').length === 4"));
  expect("the order survives a reload", await order(page, 5), "d,a,c");

  // 3. Esc cancels
  const before = [status("b"), prio("b")];
  await drag(page, await where(page, card("b")), await where(page, card("c")), {
    esc: true,
  });
  expect(
    "Esc cancels the drag",
    [status("b"), prio("b"), await page.evaluate("!!document.querySelector('.kghost')")],
    [...before, false],
  );

  // 4. dropping outside the columns cancels (over the top bar)
  await drag(page, await where(page, card("b")), { x: 200, y: 4 });
  expect("dropping outside cancels", [status("b"), await order(page, 2)], ["todo", "b"]);

  // 5. keyboard: b -> next column, announced, focus kept
  await page.evaluate(`document.querySelector('${card("b")}').focus()`);
  await key(page, " ", "Space", 32, " ");
  expect(
    "Space picks the card up and announces it",
    await page.evaluate("document.querySelector('[role=status][aria-live=assertive]').textContent.length > 0"),
    true,
  );
  await key(page, "ArrowRight", "ArrowRight", 39);
  await key(page, " ", "Space", 32, " ");
  await sleep(1200);
  expect("keyboard moves b to in progress", [status("b"), await order(page, 3)], ["in-progress", "b"]);
  expect("focus returns to the moved card", await page.evaluate("document.activeElement?.dataset?.id"), "b");
  page.close();
} catch (e) {
  failed = e;
  console.error(`FAIL e2e-kanban: ${e.stack ?? e}`);
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
console.log("e2e-kanban: all scenarios passed");
