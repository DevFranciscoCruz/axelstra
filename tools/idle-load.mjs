// Idle-load check of the daemon: does a daemon with an app attached and nothing happening stay quiet?
//
//   cargo build -p axelstra && node tools/idle-load.mjs [--bin path] [--worktrees 8] [--sessions 2] [--chatty 1] [--idle 45] [--max-spawns-per-min 2] [--json]
//
// Boots an isolated daemon (same isolation as tools/smoke.mjs, plus AXELSTRA_TESTING=1 for `debug.load`), makes a temp git
// repo with N worktrees, connects like the app (hello, watch.subscribe, session.subscribe-less), calls the polled methods
// once, then stays idle and reports per method: calls per minute (daemon counter, `debug.load`), child processes started
// (`core::proc::hidden` counter) and new `lag.log` lines. The app is not involved: a call the app's loaders make at idle
// shows up only when something in the daemon triggers them, so this measures what the DAEMON does on its own and what its
// events (`session.changed`, `resources.changed`) per minute would feed the app's loaders with.
//
// Exit 1 when the idle window starts more than --max-spawns-per-min child processes per minute, writes any lag.log line,
// or any method is called by the daemon itself (not by this script). Exit 2 when the binary is missing.
import { spawn, spawnSync } from "node:child_process";
import { existsSync, mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import net from "node:net";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { realTmpdir } from "./lib/tmp.mjs";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const exe = process.platform === "win32" ? ".exe" : "";
const argv = process.argv.slice(2);
const opt = (name, def) => {
  const i = argv.indexOf(`--${name}`);
  return i >= 0 ? argv[i + 1] : def;
};
const asJson = argv.includes("--json");
const worktrees = Number(opt("worktrees", 8));
const idleSecs = Number(opt("idle", 45));
const maxSpawns = Number(opt("max-spawns-per-min", 2));
const bin = resolve(opt("bin", process.env.AXELSTRA_BIN ?? join(root, "target", "debug", `axelstra${exe}`)));
if (!existsSync(bin)) {
  console.error(`idle-load: binary not found: ${bin} (cargo build -p axelstra)`);
  process.exit(2);
}

const dir = mkdtempSync(join(realTmpdir(), "axelstra-idle-"));
const home = join(dir, "home");
const data = join(home, "axelstra");
mkdirSync(home, { recursive: true });
const suffix = `idle${process.pid}`;
const env = {
  ...process.env,
  HOME: home,
  USERPROFILE: home,
  LOCALAPPDATA: join(home, "local"), // not data: the real default dir must differ from the explicit one (AXELSTRA_TESTING)
  AXELSTRA_DATA_DIR: data,
  XDG_DATA_HOME: join(home, "share"),
  USERNAME: `axidle${process.pid}`,
  USER: `axidle${process.pid}`,
  AXELSTRA_TEST_PIPE: suffix,
  AXELSTRA_TESTING: "1",
};
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const git = (cwd, ...args) => {
  const r = spawnSync("git", args, { cwd, encoding: "utf8" });
  if (r.status !== 0) throw new Error(`git ${args.join(" ")}: ${r.stderr}`);
  return r.stdout;
};

// the pipe name, as proto::pipe_name builds it (Windows: user SID + FNV-1a of the data dir; elsewhere a socket in it)
function pipePath() {
  if (process.platform !== "win32") return join(data, "axelstra.sock");
  const sid = spawnSync(
    join(process.env.SystemRoot ?? "C:/Windows", "System32", "whoami.exe"),
    ["/user", "/fo", "csv", "/nh"],
    { encoding: "utf8" },
  ).stdout.match(/S-1-[\d-]+/)?.[0];
  if (!sid) throw new Error("cannot read the user SID");
  let h = 0xcbf29ce484222325n;
  const key = data.toLowerCase().replaceAll("\\", "/").replace(/\/+$/, "");
  for (const b of Buffer.from(key)) h = ((h ^ BigInt(b)) * 0x100000001b3n) & 0xffffffffffffffffn;
  return `\\\\.\\pipe\\axelstra-${sid}-d${h.toString(16).padStart(16, "0")}-t${suffix}`;
}

class Conn {
  constructor(sock) {
    this.sock = sock;
    this.buf = Buffer.alloc(0);
    this.next = 1;
    this.pending = new Map();
    this.events = 0;
    this.eventKinds = {};
    sock.on("data", (d) => {
      this.buf = Buffer.concat([this.buf, d]);
      for (;;) {
        if (this.buf.length < 5) return;
        const n = this.buf.readUInt32LE(0);
        if (this.buf.length < 4 + n) return;
        const kind = this.buf[4];
        const payload = this.buf.subarray(5, 4 + n);
        this.buf = this.buf.subarray(4 + n);
        if (kind === 0) {
          const m = JSON.parse(payload.toString());
          const p = this.pending.get(m.id);
          if (p) {
            this.pending.delete(m.id);
            m.error !== undefined ? p.reject(new Error(`${m.code ?? ""} ${m.error}`)) : p.resolve(m.result);
          }
        } else if (kind === 3) {
          this.events++;
          try {
            const e = JSON.parse(payload.toString());
            const k = e.event ?? e.type ?? e.method ?? "?";
            this.eventKinds[k] = (this.eventKinds[k] ?? 0) + 1;
          } catch {}
        }
      }
    });
  }
  call(method, params = {}) {
    const id = this.next++;
    const body = Buffer.from(JSON.stringify({ id, method, params }));
    const head = Buffer.alloc(5);
    head.writeUInt32LE(body.length + 1, 0);
    head[4] = 0;
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      this.sock.write(Buffer.concat([head, body]));
    });
  }
}

const connect = (path) =>
  new Promise((res, rej) => {
    const s = net.connect(path);
    s.once("connect", () => res(new Conn(s)));
    s.once("error", rej);
  });

const lagLines = () => {
  try {
    return readFileSync(join(data, "lag.log"), "utf8").split("\n").filter(Boolean);
  } catch {
    return [];
  }
};

let daemon;
let failed = null;
let report = null;
try {
  // repo with N worktrees (git directly: the daemon lists them from git)
  const repo = join(dir, "repo");
  mkdirSync(repo);
  git(repo, "init", "-q", "-b", "main");
  git(repo, "config", "user.email", "t@t");
  git(repo, "config", "user.name", "t");
  writeFileSync(join(repo, "a.txt"), "x\n");
  git(repo, "add", "a.txt");
  git(repo, "commit", "-qm", "init");
  const wts = [];
  for (let i = 0; i < worktrees; i++) {
    const p = join(dir, "repo.axelstra-wt", `w${i}`);
    git(repo, "worktree", "add", "-q", "-b", `ax/w${i}`, p);
    wts.push(p);
  }

  daemon = spawn(bin, ["daemon"], {
    env,
    stdio: ["ignore", "ignore", "inherit"],
  });
  daemon.on("exit", (c) => (daemon.gone = c ?? "signal"));
  let conn = null;
  for (let i = 0; i < 80 && !conn; i++) {
    if (daemon.gone !== undefined) throw new Error(`daemon exited early (${daemon.gone})`);
    try {
      conn = await connect(pipePath());
    } catch {
      await sleep(500);
    }
  }
  if (!conn) throw new Error("daemon did not accept a connection in 40s");
  await conn.call("hello", {
    token: readFileSync(join(data, "token"), "utf8").trim(),
    client: "idle-load",
  });

  const project = await conn.call("project.add", { path: repo });
  const pid = project.id;
  // like the app after start: the lists it draws, once
  const paths = [repo, ...wts];
  const once = [
    ["project.list", {}],
    ["session.list", {}],
    ["worktree.overview", {}],
    ["dispatch.overlap_files", {}],
    ["agent.worktree_usage", { worktrees: paths }],
    ["budget.status", { worktrees: paths }],
    ["gh.list", { project: pid, kind: "pr" }],
    ["settings.get", {}],
    ...wts.slice(0, 3).flatMap((w) => [
      ["git.changes", { worktree: w }],
      ["worktree.status", { worktree: w }],
      ["ports.list", { worktree: w }],
    ]),
  ];
  for (const [m, p] of once) await conn.call(m, p).catch(() => {}); // a method that needs gh or a login may fail: it still ran
  await conn.call("watch.subscribe", { worktree: wts[0] ?? repo }).catch(() => {});
  // sessions are what makes the daemon emit session.changed/resources.changed: --sessions N idle shells, plus --chatty M
  // sessions that print a line every second (an agent at work), the case behind the app's 1/s polling
  const spawnOne = (label, program, args, cwd) =>
    conn
      .call("session.spawn", { program, args, label, cwd, cols: 80, rows: 24 })
      .catch((e) => console.error(`spawn: ${e.message}`));
  const shell = process.platform === "win32" ? "cmd.exe" : "sh";
  for (let i = 0; i < Number(opt("sessions", 2)); i++) await spawnOne(`idle${i}`, shell, [], wts[i] ?? repo);
  for (let i = 0; i < Number(opt("chatty", 1)); i++)
    await spawnOne(`chatty${i}`, process.execPath, ["-e", "setInterval(()=>console.log('tick'),1000)"], wts[i] ?? repo);
  await sleep(5000); // let what the calls above set off (watchers, first looks) settle

  const live = await conn.call("session.list").catch(() => []);
  if (!asJson)
    console.log(
      `sessions running: ${Array.isArray(live) ? live.length : "?"}, events since start: ${conn.events} ${JSON.stringify(conn.eventKinds)}`,
    );
  const snap = () => conn.call("debug.load");
  const before = await snap();
  const lagBefore = lagLines().length;
  const evBefore = { n: conn.events, kinds: { ...conn.eventKinds } };
  const t0 = Date.now();
  await sleep(idleSecs * 1000);
  const after = await snap();
  const mins = (Date.now() - t0) / 60000;
  const lagNew = lagLines().slice(lagBefore);

  const methods = {};
  for (const [m, v] of Object.entries(after.calls)) {
    if (m === "debug.load") continue;
    const calls = v.calls - (before.calls[m]?.calls ?? 0);
    const ms = v.ms - (before.calls[m]?.ms ?? 0);
    if (calls > 0)
      methods[m] = {
        calls,
        per_min: +(calls / mins).toFixed(1),
        avg_ms: Math.round(ms / calls),
      };
  }
  const spawns = after.spawns - before.spawns;
  const kinds = {};
  for (const [k, n] of Object.entries(conn.eventKinds)) {
    const d = n - (evBefore.kinds[k] ?? 0);
    if (d > 0) kinds[k] = +(d / mins).toFixed(1);
  }
  report = {
    worktrees,
    idle_secs: idleSecs,
    daemon_calls_per_min: methods, // nobody called these during the window: the daemon (or the watch subscriber) did
    spawns: {
      total: spawns,
      per_min: +(spawns / mins).toFixed(1),
      limit_per_min: maxSpawns,
    },
    events_per_min: kinds,
    lag_log_new_lines: lagNew.length,
    lag_log_sample: lagNew.slice(0, 5),
  };
  const problems = [];
  if (report.spawns.per_min > maxSpawns) problems.push(`spawns ${report.spawns.per_min}/min > ${maxSpawns}`);
  if (lagNew.length > 0) problems.push(`${lagNew.length} new lag.log lines at idle`);
  if (Object.keys(methods).length > 0) problems.push(`daemon calls at idle: ${Object.keys(methods).join(", ")}`);
  report.ok = problems.length === 0;
  report.problems = problems;
  if (problems.length) failed = new Error(problems.join("; "));
} catch (e) {
  failed = e;
} finally {
  if (daemon && daemon.gone === undefined) daemon.kill();
  await sleep(300);
  try {
    rmSync(dir, { recursive: true, force: true });
  } catch {}
}

if (report) {
  if (asJson) console.log(JSON.stringify(report, null, 2));
  else {
    console.log(`idle-load: ${report.worktrees} worktrees, ${report.idle_secs}s idle`);
    console.log(`  child processes: ${report.spawns.total} (${report.spawns.per_min}/min, limit ${maxSpawns})`);
    console.log(`  lag.log new lines: ${report.lag_log_new_lines}`);
    console.log("  event (to subscriber)   per min");
    for (const [k, v] of Object.entries(report.events_per_min)) console.log(`    ${k.padEnd(22)} ${v}`);
    console.log("  method (called at idle)  per min   avg ms");
    for (const [m, v] of Object.entries(report.daemon_calls_per_min))
      console.log(`    ${m.padEnd(22)} ${String(v.per_min).padStart(8)} ${String(v.avg_ms).padStart(8)}`);
  }
}
if (failed) {
  console.error(`idle-load FAILED: ${failed.message}`);
  process.exit(1);
}
console.log("idle-load passed");
