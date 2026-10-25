// Visual check of the real desktop app (Windows, WebView2) over the Chrome DevTools Protocol: screenshots of the
// Welcome screen, Settings and a sidebar with lead rows and a stalled agent, in every language and in a small window.
//
//   cargo build -p axelstra && cargo build -p axelstra-app --features tauri/custom-protocol   (frontend built in app/)
//   node tools/visual.mjs [outDir] [path\to\axelstra-app.exe]
//
// Same isolation as tools/e2e.mjs: a throwaway data dir and WebView2 profile, never the developer's own daemon.
// The "agent" is a .cmd stand-in that only sleeps, so no real agent CLI or network is involved.
import { spawn, spawnSync } from "node:child_process";
import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { layoutProblems, resizeClient, windowInfo } from "./winrect.mjs";
import { freePort } from "./lib/freeport.mjs";

if (process.platform !== "win32") {
  console.log("visual: WebView2 only; skipping");
  process.exit(0);
}
const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const out = resolve(process.argv[2] ?? join(tmpdir(), "axelstra-visual"));
const exe = resolve(
  process.argv[3] ?? join(root, "target", "debug", "axelstra-app.exe"),
);
const cliBin = join(dirname(exe), "axelstra.exe");
for (const f of [exe, cliBin]) {
  if (!existsSync(f)) {
    console.error(`visual: missing ${f}`);
    process.exit(2);
  }
}
mkdirSync(out, { recursive: true });

const dir = mkdtempSync(join(tmpdir(), "axelstra-vis-"));
const home = join(dir, "home");
const bin = join(dir, "bin");
mkdirSync(home, { recursive: true });
mkdirSync(bin, { recursive: true });
writeFileSync(
  join(bin, "fakeagent.cmd"),
  "@echo off\r\nping -n 900 127.0.0.1 >nul\r\n",
);
mkdirSync(join(home, "axelstra"), { recursive: true });
writeFileSync(join(home, "axelstra", "settings.json"), JSON.stringify({ stall_minutes: 1 }));
const port = await freePort();
const user = `axvis${process.pid}`;
const env = {
  ...process.env,
  HOME: home,
  LOCALAPPDATA: home,
  AXELSTRA_DATA_DIR: join(home, "axelstra"),
  XDG_DATA_HOME: join(home, "share"),
  USERNAME: user,
  USER: user,
  AXELSTRA_TEST_PIPE: user,
  PATH: `${bin};${process.env.PATH}`,
  AXELSTRA_TEST_HIDDEN: process.env.AXELSTRA_TEST_HIDDEN ?? "1", // the host shows its window transparent, click-through, unfocusable
  WEBVIEW2_USER_DATA_FOLDER: join(dir, "webview"),
  // the host (debug builds) hands the port to WebView2 itself; the env var alone is ignored by runtime 155
  AXELSTRA_TEST_DEVTOOLS_PORT: String(port),
  WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port}`,
};
const cli = (...a) =>
  spawnSync(cliBin, a, { env, encoding: "utf8", timeout: 30000 });
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const kill = (pid) =>
  spawnSync("taskkill", ["/PID", String(pid), "/T", "/F"], { stdio: "ignore" });

let app;
async function connect() {
  let target;
  for (let i = 0; i < 80 && !target; i++) {
    try {
      const list = await (
        await fetch(`http://127.0.0.1:${port}/json/list`)
      ).json();
      target = list.find(
        (t) =>
          t.type === "page" &&
          t.webSocketDebuggerUrl &&
          t.url !== "about:blank",
      );
    } catch {
      /* not listening yet */
    }
    if (!target) await sleep(500);
  }
  if (!target) throw new Error("no page target");
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
  await send("Runtime.enable");
  const evaluate = async (expression) => {
    const r = await send("Runtime.evaluate", {
      expression,
      returnByValue: true,
      awaitPromise: true,
    });
    if (r.exceptionDetails)
      throw new Error(
        r.exceptionDetails.exception?.description ?? "evaluate failed",
      );
    return r.result.value;
  };
  const shot = async (name) => {
    await sleep(700);
    const r = await send("Page.captureScreenshot", { format: "png" });
    writeFileSync(join(out, `${name}.png`), Buffer.from(r.data, "base64"));
    console.log(`shot ${name}`);
  };
  // a real resize of the host window: CDP Browser.setWindowBounds moves WebView2's inner window instead (see tools/winrect.mjs)
  const size = async (width, height) => {
    const k = windowInfo(app.pid).dpi / 96;
    resizeClient(app.pid, Math.round(width * k), Math.round(height * k));
    await sleep(700); // the webview follows the host window asynchronously
    const bad = layoutProblems(windowInfo(app.pid));
    if (bad.length)
      throw new Error(`window layout is off after resizing: ${bad.join("; ")}`);
    await sleep(500);
  };
  return { send, evaluate, shot, size, close: () => ws.close() };
}

async function until(label, fn, ms = 30000) {
  const t0 = Date.now();
  while (Date.now() - t0 < ms) {
    try {
      const v = await fn();
      if (v) return v;
    } catch {
      /* retry */
    }
    await sleep(400);
  }
  throw new Error(`timeout waiting for ${label}`);
}

const clickText = (re) =>
  `(() => { const b = [...document.querySelectorAll('button')].find((e) => ${re}.test(e.innerText.trim())); if (!b) return false; b.click(); return true; })()`;
let hasProject = false;
const LOCALES = ["pt-PT", "en", "pt-BR"];
const setLocale = async (page, l) => {
  // the daemon's copy of the UI state wins over localStorage at startup, so the language goes there too
  writeFileSync(
    join(home, "axelstra", "settings.json"),
    JSON.stringify({ stall_minutes: 1, ui: { "axelstra.locale": l } }),
  );
  await page.evaluate(`localStorage.setItem('axelstra.locale', '${l}')`);
  await page.send("Page.reload");
  await until("the UI", () =>
    page.evaluate("document.getElementById('root')?.children.length > 0"),
  );
  await sleep(1200);
  if (hasProject)
    await until("the project", () =>
      page.evaluate("document.body.innerText.includes('demo')"),
    );
};

let failed = null;
try {
  app = spawn(exe, [], { env, stdio: "ignore" });
  const page = await connect();
  await until("the UI", () =>
    page.evaluate("document.getElementById('root')?.children.length > 0"),
  );
  await until("the daemon", () => cli("ls").status === 0);
  await page.size(1280, 800);

  // 0. the Select dropdowns (dev gallery at #ui): agent logos in colour, rows aligned, light and dark, wide and small
  if (process.env.VISUAL_SELECT) {
    const key = async (k, code) => {
      for (const type of ["keyDown", "keyUp"])
        await page.send("Input.dispatchKeyEvent", {
          type,
          key: k,
          code: k,
          windowsVirtualKeyCode: code,
        });
    };
    await page.evaluate("location.hash = '#ui'; location.reload()");
    await sleep(1500);
    await until("the gallery", () =>
      page.evaluate("document.querySelectorAll('.select-btn').length > 0"),
    );
    for (const [w, h] of [
      [1280, 800],
      [800, 600],
    ]) {
      await page.size(w, h);
      for (const theme of ["dark", "light"]) {
        await page.evaluate(
          `document.documentElement.dataset.theme = '${theme}'`,
        );
        for (const [i, name] of ["plain", "agents"].entries()) {
          await page.evaluate(
            `document.querySelectorAll('.select-btn')[${i}].scrollIntoView({ block: 'center' }); document.querySelectorAll('.select-btn')[${i}].click()`,
          );
          await sleep(500);
          const probe = await page.evaluate(
            "(() => { const l = document.querySelector('[role=listbox]'); if (!l) return 'no listbox'; const c = l.parentElement.getBoundingClientRect(); const it = l.querySelector('[role=option]').getBoundingClientRect(); const cs = getComputedStyle(l); const ic = [...l.querySelectorAll('[data-agent]')].map((e) => e.dataset.agent + '=' + getComputedStyle(e).color).join(' '); return 'ul padL=' + cs.paddingLeft + ' item.left-content.left=' + Math.round(it.left - c.left) + ' ' + ic; })()",
          );
          console.log(`select ${name} ${theme} ${w}: ${probe}`);
          await page.shot(`select-${name}-${theme}-${w}`);
          await key("Escape", 27);
          await sleep(300);
        }
      }
    }
    if (process.env.VISUAL_SELECT === "only") {
      page.close();
      throw Object.assign(new Error("select only"), { benign: true });
    }
    await page.evaluate("location.hash = ''; location.reload()");
    await page.size(1280, 800);
  }

  // 1. Welcome: no project yet
  for (const l of LOCALES) {
    await setLocale(page, l);
    await page.shot(`welcome-${l}`);
  }

  // 1a. the status bar "shortcuts" button: its kbd must sit inside the hover box
  if (process.env.VISUAL_SB) {
    for (const l of LOCALES) {
      await setLocale(page, l);
      await until("the shortcuts button", () =>
        page.evaluate(
          "!!document.querySelector('.statusbar button .hint-keys')",
        ),
      );
      const r = await page.evaluate(
        "(()=>{const b=document.querySelector('.statusbar button .hint-keys').closest('button').getBoundingClientRect();const k=document.querySelector('.statusbar button kbd').getBoundingClientRect();return JSON.stringify({b:[b.top,b.bottom,b.left,b.right],k:[k.top,k.bottom,k.left,k.right]})})()",
      );
      const { b, k } = JSON.parse(r);
      console.log(`sb-${l}: button ${b} kbd ${k}`);
      if (k[0] < b[0] || k[1] > b[1] || k[2] < b[2] || k[3] > b[3])
        throw new Error(`statusbar kbd outside hover box (${l})`);
      await page.shot(`statusbar-${l}`);
    }
    if (process.env.VISUAL_SB === "only") {
      page.close();
      throw Object.assign(new Error("statusbar only"), { benign: true });
    }
  }

  // 1b. the Add project dialog (six origins as tabs: local, clone, new) in a small window, every language
  if (process.env.VISUAL_ADD) {
    await page.size(800, 600);
    const esc = async () => {
      for (const type of ["keyDown", "keyUp"])
        await page.send("Input.dispatchKeyEvent", {
          type,
          key: "Escape",
          code: "Escape",
          windowsVirtualKeyCode: 27,
        });
      await sleep(400);
    };
    for (const l of LOCALES) {
      await setLocale(page, l);
      await until("the add button", () =>
        page.evaluate(clickText("/^[+] (Adicionar projeto|Add project)$/")),
      );
      await until("the dialog", () =>
        page.evaluate("!!document.querySelector('[role=dialog]')"),
      );
      // the origins are tabs, in the same order in every language: 0 local, 1 clone, 2 GitHub, 3 new, 4 remote, 5 discovered
      const tab = (i) =>
        page.evaluate(
          `(() => { const t = document.querySelectorAll('[role=dialog] [role=tab]')[${i}]; if (!t) return false; t.click(); return true; })()`,
        );
      await page.shot(`addproject-local-${l}`);
      for (const [i, name] of [
        [1, "clone"],
        [3, "new"],
      ]) {
        await until(`the ${name} tab`, () => tab(i));
        await sleep(900);
        await page.shot(`addproject-${name}-${l}`);
      }
      await esc();
    }
    await page.size(1280, 800);
    if (process.env.VISUAL_ADD === "only") {
      page.close();
      throw Object.assign(new Error("add project only"), { benign: true });
    }
  }

  // 2. a project with a lead and a worker per worktree; one is quiet long enough to be stalled
  const repo = join(dir, "demo");
  mkdirSync(repo);
  const git = (...a) => spawnSync("git", a, { cwd: repo, encoding: "utf8" });
  git("init", "-q", "-b", "main");
  git(
    "-c",
    "user.name=v",
    "-c",
    "user.email=v@example.invalid",
    "commit",
    "-q",
    "--allow-empty",
    "-m",
    "init",
  );
  const must = (r, what) => {
    if (r.status !== 0)
      throw new Error(`${what} failed: ${r.stderr || r.stdout}`);
    return r.stdout;
  };
  must(cli("add-project", repo), "add-project");
  const wt = must(
    cli("worktree", "create", "demo", "feature-x"),
    "worktree create",
  )
    .trim()
    .split(/\r?\n/)
    .pop();
  must(
    cli("agent", wt, "--agent", "fakeagent", "--prompt", "lead"),
    "lead agent",
  );
  must(
    cli("agent", wt, "--agent", "fakeagent", "--prompt", "worker"),
    "worker agent",
  );
  const wt2 = must(
    cli(
      "worktree",
      "create",
      "demo",
      "bugfix-long-name-for-the-narrow-sidebar",
    ),
    "worktree create 2",
  )
    .trim()
    .split(/\r?\n/)
    .pop();
  must(
    cli("agent", wt2, "--agent", "fakeagent", "--prompt", "lead"),
    "lead agent 2",
  );
  hasProject = true;
  await page.send("Page.reload");
  await until("the project", () =>
    page.evaluate("document.body.innerText.includes('demo')"),
  );
  if (!process.env.VISUAL_FAST)
    console.log("waiting for the stall watchdog (stall_minutes=1)...");
  if (!process.env.VISUAL_FAST)
    await until(
      "a stalled agent",
      () => JSON.parse(cli("ls", "--json").stdout).some((s) => s.stalled),
      240000,
    ).catch(() =>
      console.log(
        "no agent flagged as stalled within 4 minutes; the hourglass shot will not show it",
      ),
    );
  await sleep(11000); // the sidebar polls every 10 s

  // 2a. status bar popovers (bell, sessions, branches, ports...): open, fully visible (not clipped by the bar), inside the window
  if (process.env.VISUAL_BELL) {
    const seed = JSON.stringify(
      [
        ["done", "Tarefa 1 done"],
        ["waiting", "Agent asks something"],
        ["ci-fail", "CI #12 failed"],
      ].map(([kind, text], i) => ({
        id: `v${i}`,
        at: Date.now() - i * 60000,
        kind,
        worktree: wt,
        text,
        read: false,
      })),
    );
    const key = (k, code) =>
      ["keyDown", "keyUp"].map((type) =>
        page.send("Input.dispatchKeyEvent", {
          type,
          key: k,
          code: k,
          windowsVirtualKeyCode: code,
        }),
      );
    // the first visible popover/dialog under the status bar, as {ok, why, text}
    const probe = (sel) =>
      page.evaluate(`(() => {
        const p = document.querySelector(${JSON.stringify(sel)});
        if (!p) return { ok: false, why: 'absent' };
        const r = p.getBoundingClientRect();
        if (!r.width || !r.height) return { ok: false, why: 'empty' };
        if (r.left < 0 || r.top < 0 || r.right > innerWidth || r.bottom > innerHeight) return { ok: false, why: 'outside ' + [r.left, r.top, r.right, r.bottom].map(Math.round) };
        const at = document.elementFromPoint(r.left + r.width / 2, r.top + Math.min(r.height / 2, 20));
        if (!at || !p.contains(at)) return { ok: false, why: 'covered or clipped by ' + (at && at.className) };
        return { ok: true, text: p.innerText };
      })()`);
    const outside = async () => {
      for (const type of ["mousePressed", "mouseReleased"])
        await page.send("Input.dispatchMouseEvent", {
          type,
          x: 400,
          y: 300,
          button: "left",
          clickCount: 1,
        });
      await sleep(300);
    };
    for (const l of LOCALES) {
      await page.evaluate(
        `localStorage.setItem('axelstra.notifications', ${JSON.stringify(seed)})`,
      );
      await setLocale(page, l);
      await until("the bell", () =>
        page.evaluate("!!document.querySelector('.statusbar .notif button')"),
      );
      const bell = "document.querySelector('.statusbar .notif button').click()";
      for (const round of [1, 2]) {
        await page.evaluate(bell);
        await sleep(400);
        const r = await probe(".statusbar .notif-pop");
        if (!r.ok)
          throw new Error(`bell popover (${l}, open #${round}): ${r.why}`);
        for (const t of [
          "Tarefa 1 done",
          "Agent asks something",
          "CI #12 failed",
        ])
          if (!r.text.includes(t))
            throw new Error(`bell popover (${l}) lacks entry "${t}"`);
        if (round === 1) await page.shot(`bell-${l}`);
        if (round === 1) await Promise.all(key("Escape", 27));
        else await outside();
        await sleep(300);
        if (await page.evaluate("!!document.querySelector('.notif-pop')"))
          throw new Error(
            `bell popover (${l}) did not close with ${round === 1 ? "Esc" : "an outside click"}`,
          );
      }
      // every other chip that owns a popover (sessions, branches, ports...)
      const n = await page.evaluate(
        "document.querySelectorAll('.statusbar .sb-pop-wrap > .sb-item').length",
      );
      for (let i = 0; i < n; i++) {
        await page.evaluate(
          `document.querySelectorAll('.statusbar .sb-pop-wrap > .sb-item')[${i}].click()`,
        );
        await sleep(400);
        const r = await probe(".statusbar .sb-pop");
        if (!r.ok) throw new Error(`status bar popover #${i} (${l}): ${r.why}`);
        await outside();
      }
    }
    if (process.env.VISUAL_BELL === "only") {
      page.close();
      throw Object.assign(new Error("bell only"), { benign: true });
    }
  }

  for (const l of LOCALES) {
    await setLocale(page, l);
    await page.shot(`sidebar-${l}`);
    // the version chip in the status bar opens Settings scrolled to About
    await page.shot(`footer-${l}`);
    await page.evaluate(
      "document.querySelector('[data-testid=version-chip]')?.click()",
    );
    await until("the About section", () =>
      page.evaluate(
        "!!document.querySelector('[data-testid=about-warn], [data-testid=about-sync]')",
      ),
    );
    await sleep(600);
    await page.shot(`about-${l}`);
    console.log(
      `about-${l}: ${await page.evaluate("document.querySelector('[data-testid=about]')?.innerText.split(String.fromCharCode(10)).join(' | ')")}`,
    );
    for (const type of ["keyDown", "keyUp"])
      await page.send("Input.dispatchKeyEvent", {
        type,
        key: "Escape",
        code: "Escape",
        windowsVirtualKeyCode: 27,
      });
    await sleep(400);
    await page.evaluate(clickText("/^(Definições|Settings|Configurações)$/"));
    await sleep(900);
    await page.shot(`settings-${l}`);
    const over = await page.evaluate(
      "(() => { const d = document.querySelector('[role=dialog]'); if (!d) return 'no dialog'; const out = []; const dr = d.getBoundingClientRect().right; for (const e of d.querySelectorAll('*')) { const r = e.getBoundingClientRect(); if (r.width && r.right > dr + 1) out.push('RIGHT ' + e.tagName + '.' + e.className + ' ' + Math.round(r.right - dr)); if (e.scrollWidth > e.clientWidth + 1 && getComputedStyle(e).overflowX !== 'visible') out.push(e.tagName + '.' + e.className + ' ' + e.scrollWidth + '>' + e.clientWidth); } return d.scrollWidth + '/' + d.clientWidth + ' ' + out.slice(0, 5).join(' | '); })()",
    );
    console.log(`overflow in settings-${l}: ${over}`);
    await page.send("Input.dispatchKeyEvent", {
      type: "keyDown",
      key: "Escape",
      code: "Escape",
      windowsVirtualKeyCode: 27,
    });
    await page.send("Input.dispatchKeyEvent", {
      type: "keyUp",
      key: "Escape",
      code: "Escape",
      windowsVirtualKeyCode: 27,
    });
  }

  // 3. the smallest window the app allows
  await page.size(800, 600);
  for (const l of LOCALES) {
    await setLocale(page, l);
    await page.shot(`small-sidebar-${l}`);
    await page.evaluate(clickText("/^(Definições|Settings|Configurações)$/"));
    await sleep(900);
    await page.shot(`small-settings-${l}`);
    await page.send("Input.dispatchKeyEvent", {
      type: "keyDown",
      key: "Escape",
      code: "Escape",
      windowsVirtualKeyCode: 27,
    });
  }

  // 4. the right panel at its minimum width (300): every tab, the CI toggle of the timeline and the status bar
  if (!process.env.VISUAL_SKIP_PANEL) {
    await page.size(1280, 800);
    await page.evaluate(
      "localStorage.setItem('axelstra.right', '1'); localStorage.setItem('axelstra.rightW', '300')",
    );
    await setLocale(page, "pt-PT");
    await page.evaluate(clickText("/feature-x/"));
    await until("the right panel", () =>
      page.evaluate("!!document.getElementById('rpanel')"),
    );
    const panelWidth = () =>
      page.evaluate(
        "(() => { const p = document.getElementById('rpanel'); const r = p.getBoundingClientRect(); const bad = [...p.querySelectorAll('*')].filter((e) => { const b = e.getBoundingClientRect(); return b.width && (b.right > r.right + 1 || b.left < r.left - 1); }).map((e) => e.tagName + '.' + e.className); return Math.round(r.width) + 'px, outside: ' + bad.slice(0, 5).join(' | '); })()",
      );
    for (const tab of ["files", "changes", "pulls", "timeline"]) {
      await page.evaluate(`document.getElementById('rtab-${tab}')?.click()`);
      await sleep(900);
      await page.shot(`panel-${tab}`);
      console.log(`panel ${tab}: ${await panelWidth()}`);
    }
    await page.evaluate(clickText("/^Incluir CI$/"));
    await sleep(2500);
    await page.shot("panel-timeline-ci");
    console.log(`panel timeline-ci: ${await panelWidth()}`);
    await page.evaluate(
      "document.querySelector('.statusbar')?.scrollIntoView()",
    );
    await page.shot("statusbar");
  }
  page.close();
} catch (e) {
  if (!e.benign) failed = e;
  if (!e.benign) console.error(`FAIL visual: ${e.stack ?? e}`);
} finally {
  if (app) kill(app.pid);
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
    /* a lingering handle */
  }
}
if (failed) process.exit(1);
console.log(`screenshots in ${out}`);
