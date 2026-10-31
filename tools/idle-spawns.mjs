// Spawn storm check: how many child processes does a daemon start per minute when a realistic desktop sits near idle?
//
//   cargo build -p axelstra && node tools/idle-spawns.mjs [--bin path] [--projects 3] [--worktrees 10] [--agents 8]
//     [--terminals 2] [--warmup 30] [--idle 180] [--agent-every 20] [--focus-every 120] [--commit-every 5] [--working 2|all]
//     [--churn-every 90] [--no-remote]
//     [--max-spawns-per-min N] [--json]
//
// Boots an isolated daemon (temp AXELSTRA_DATA_DIR, own pipe, AXELSTRA_TESTING=1; never the real data dir or pipe), makes
// --projects throwaway git repos with --worktrees worktrees spread over them (`worktree.create`, auto like dispatch makes
// them, every fourth manual; some dirty, some with commits not yet in main), each repo with a github.com `origin` the daemon cannot reach: gh is kept logged out with an empty GH_CONFIG_DIR),
// adds them, starts --agents agent sessions (a shell that prints its own agent credential, one project main per repo,
// the rest in worktrees) and --terminals plain shells. Then it behaves like the app and the agents for --warmup + --idle
// seconds:
//   - app: one watch connection (`watch.subscribe` on the selected worktree) and the loaders the frontend runs on each
//     event, with its debounces: session.list (session.changed), worktree.overview (projects/dispatch/janitor/pr events,
//     session.changed only when the session set moved), dispatch.overlap_files (fs/projects/dispatch), project.list then
//     worktree.status (projects.changed, at most 1 per 5 s) and gh.list (at most 1 per 3 min), git.changes of the
//     selected worktree (fs.changed, at most 1/s), agent.worktree_usage and budget.status (usage.changed); every
//     --focus-every seconds a window focus (project.list, gh.pr_refresh, session.list).
//   - agents: every --agent-every seconds (jittered) each agent's hooks say working then idle (`session.hint`), and in
//     turn it reads its inbox (`msg.inbox`), sends its project main a quiet message (`msg.send`) or lists sessions.
//     --working N of the agents in worktrees (default 2, `all` for a busy desktop, 0 for none) also work there like a
//     coding agent: each turn they edit a file and run `git status`, every --commit-every turns they commit (these git
//     runs are the script's, not counted as the daemon's). The others only do their bookkeeping: most agents near idle.
//   - dispatch: every --churn-every seconds (0 = never) a task comes and goes like `axelstra dispatch new` makes it: a
//     worktree (`worktree.create`, auto: projects.changed) and a worker in it that ends a few seconds later (its exit is
//     dispatch.changed and wakes the janitor, which may then remove the empty worktree: projects.changed, janitor.events).
// After the warm-up it reads `debug.load` and `mem.status` (breakdown.own.sites), again at the end, and prints the child
// processes started per minute by (site, command) over the idle window, plus the calls per method in that window.
//
// Exit 1 when the window starts more than --max-spawns-per-min children per minute (no limit when not given), 2 when
// the binary is missing.
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
const nProjects = Math.max(1, Number(opt("projects", 3)));
const nWorktrees = Number(opt("worktrees", 10));
const nAgents = Number(opt("agents", 8));
const nTerminals = Number(opt("terminals", 2));
const warmupSecs = Number(opt("warmup", 30));
const idleSecs = Number(opt("idle", 180));
const agentEvery = Number(opt("agent-every", 20)) * 1000;
const focusEvery = Number(opt("focus-every", 120)) * 1000;
const withRemote = !argv.includes("--no-remote");
const workingOpt = opt("working", "2");
const working = workingOpt === "all" ? Infinity : Math.max(0, Number(workingOpt));
const commitEvery = Math.max(1, Number(opt("commit-every", 5)));
const churnEvery = Number(opt("churn-every", 90)) * 1000;
const maxSpawns = opt("max-spawns-per-min") === undefined ? Infinity : Number(opt("max-spawns-per-min"));
const bin = resolve(opt("bin", process.env.AXELSTRA_BIN ?? join(root, "target", "debug", `axelstra${exe}`)));
if (!existsSync(bin)) {
  console.error(`idle-spawns: binary not found: ${bin} (cargo build -p axelstra)`);
  process.exit(2);
}

const dir = mkdtempSync(join(realTmpdir(), "axelstra-spawns-"));
const home = join(dir, "home");
const data = join(home, "axelstra");
mkdirSync(join(home, "gh"), { recursive: true });
const gitconfig = join(home, "gitconfig");
writeFileSync(gitconfig, "[user]\n\temail = t@t\n\tname = t\n[init]\n\tdefaultBranch = main\n");
const suffix = `spawns${process.pid}`;
const env = {
  ...process.env,
  HOME: home,
  USERPROFILE: home,
  LOCALAPPDATA: join(home, "local"), // not data: the real default dir must differ from the explicit one (AXELSTRA_TESTING)
  AXELSTRA_DATA_DIR: data,
  XDG_DATA_HOME: join(home, "share"),
  USERNAME: `axspawn${process.pid}`,
  USER: `axspawn${process.pid}`,
  AXELSTRA_TEST_PIPE: suffix,
  AXELSTRA_TESTING: "1",
  GIT_CONFIG_GLOBAL: gitconfig,
  GIT_TERMINAL_PROMPT: "0",
  GH_CONFIG_DIR: join(home, "gh"),
  GH_TOKEN: "",
  GITHUB_TOKEN: "",
};
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const gitAsync = (cwd, ...args) =>
  new Promise((res) => spawn("git", args, { cwd, env, stdio: "ignore" }).on("exit", res).on("error", res));
const git = (cwd, ...args) => {
  const r = spawnSync("git", args, { cwd, encoding: "utf8", env });
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
    this.onEvent = null;
    sock.on("error", () => {});
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
          try {
            this.onEvent?.(JSON.parse(payload.toString()));
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
  close() {
    this.sock.destroy();
  }
}

const connectRaw = (path) =>
  new Promise((res, rej) => {
    const s = net.connect(path);
    s.once("connect", () => res(new Conn(s)));
    s.once("error", rej);
  });
const token = () => readFileSync(join(data, "token"), "utf8").trim();
const connect = async (extra = {}) => {
  const c = await connectRaw(pipePath());
  await c.call("hello", { token: token(), client: "idle-spawns", ...extra });
  return c;
};

/** `fn` at most once per `gapMs`, after `debounceMs` of calm; calls during a run make one more after it */
function gate(fn, { debounceMs = 0, gapMs = 0 } = {}) {
  let timer = null;
  let running = false;
  let again = false;
  let last = 0;
  const run = async () => {
    timer = null;
    if (running) return void (again = true);
    running = true;
    last = Date.now();
    try {
      await fn();
    } catch {}
    running = false;
    if (again) {
      again = false;
      kick();
    }
  };
  const kick = () => {
    if (timer) return;
    const wait = Math.max(debounceMs, last + gapMs - Date.now(), 0);
    timer = setTimeout(run, wait);
  };
  kick.stop = () => timer && clearTimeout(timer);
  return kick;
}

let daemon;
let failed = null;
let report = null;
const timers = [];
const conns = [];
let stopping = false;
try {
  // projects: repo pN with its worktrees in <dir>/.axelstra-wt/pN-wK, like dispatch makes them
  const projects = [];
  for (let i = 0; i < nProjects; i++) {
    const repo = join(dir, `p${i}`);
    mkdirSync(repo);
    git(repo, "init", "-q", "-b", "main");
    writeFileSync(join(repo, "a.txt"), "x\n");
    mkdirSync(join(repo, "src"));
    writeFileSync(join(repo, "src", "lib.txt"), "lib\n");
    git(repo, "add", "a.txt", "src/lib.txt");
    git(repo, "commit", "-qm", "init");
    if (withRemote) git(repo, "remote", "add", "origin", `https://github.com/axelstra-idle-spawns/p${i}.git`);
    projects.push({ repo, wts: [] });
  }
  daemon = spawn(bin, ["daemon"], {
    env,
    stdio: ["ignore", "ignore", "inherit"],
  });
  daemon.on("exit", (c) => (daemon.gone = c ?? "signal"));
  let app = null;
  for (let i = 0; i < 80 && !app; i++) {
    if (daemon.gone !== undefined) throw new Error(`daemon exited early (${daemon.gone})`);
    try {
      app = await connect();
    } catch {
      await sleep(500);
    }
  }
  if (!app) throw new Error("daemon did not accept a connection in 40s");
  conns.push(app);
  for (const p of projects) p.id = (await app.call("project.add", { path: p.repo })).id;
  // worktrees as `axelstra dispatch new` makes them (auto, the janitor may remove them), every fourth one by hand
  for (let k = 0; k < nWorktrees; k++) {
    const p = projects[k % nProjects];
    const name = `w${k}`;
    const wt = await app.call("worktree.create", {
      project: p.id,
      name,
      ...(k % 4 === 3 ? { manual: true } : { auto: true }),
    });
    const path = wt.path ?? wt.worktree?.path;
    if (k % 3 === 1) {
      // work not integrated yet
      writeFileSync(join(path, `${name}.txt`), "work\n");
      git(path, "add", `${name}.txt`);
      git(path, "commit", "-qm", `work in ${name}`);
    }
    if (k % 3 === 2) writeFileSync(join(path, "a.txt"), "dirty\n"); // uncommitted
    p.wts.push(path);
  }
  const allPaths = projects.flatMap((p) => [p.repo, ...p.wts]);
  const selected = projects[0].wts[0] ?? projects[0].repo;

  // agents: a shell that prints its agent credential; the first of each project is its main, at the repo root
  const shell =
    process.platform === "win32"
      ? ["cmd.exe", ["/q", "/k", "echo TOK[%AXELSTRA_AGENT_TOKEN%]"]]
      : ["sh", ["-c", "echo TOK[$AXELSTRA_AGENT_TOKEN]; exec cat"]];
  const agents = [];
  let workers = 0;
  for (let i = 0; i < nAgents; i++) {
    const p = projects[i % nProjects];
    const round = Math.floor(i / nProjects);
    const cwd = round === 0 ? p.repo : (p.wts[(round - 1) % Math.max(1, p.wts.length)] ?? p.repo);
    const r = await app.call("session.spawn", {
      program: shell[0],
      args: shell[1],
      cwd,
      cols: 100,
      rows: 30,
      label: round === 0 ? "main" : `agent${i}`,
      meta: {
        kind: "agent",
        project: p.id,
        ...(round === 0 ? {} : { worktree: cwd }),
      },
    });
    const works = round > 0 && workers < working;
    if (works) workers++;
    agents.push({ id: r.id, project: p, main: round === 0, cwd, works });
  }
  for (let i = 0; i < nTerminals; i++)
    await app.call("session.spawn", {
      program: shell[0],
      args: [],
      cwd: projects[i % nProjects].wts[0] ?? projects[i % nProjects].repo,
      cols: 80,
      rows: 24,
      label: `term${i}`,
    });
  // one finished command (a worker that ended): its exit is the kind of event that wakes the janitor
  const once = process.platform === "win32" ? ["cmd.exe", ["/c", "exit 0"]] : ["sh", ["-c", "exit 0"]];
  await app.call("session.spawn", {
    program: once[0],
    args: once[1],
    cwd: projects[0].repo,
    cols: 80,
    rows: 24,
    label: "finished",
  });
  for (const a of agents) {
    const t0 = Date.now();
    for (;;) {
      const lines = JSON.stringify((await app.call("session.screen", { id: a.id, lines: 100 })).lines);
      const m = lines.match(/TOK\[([^\]%$]{32,})\]/);
      if (m) {
        a.conn = await connect({ client: "cli", agent_token: m[1] });
        conns.push(a.conn);
        break;
      }
      if (Date.now() - t0 > 15000) throw new Error(`agent ${a.id} printed no credential: ${lines.slice(0, 200)}`);
      await sleep(200);
    }
  }
  for (const a of agents) a.mainId = agents.find((b) => b.main && b.project === a.project)?.id;

  // ---- the app ----
  const watch = await connect();
  conns.push(watch);
  const appCalls = {};
  const ac = (m, p) => {
    appCalls[m] = (appCalls[m] ?? 0) + 1;
    return app.call(m, p).catch(() => {});
  };
  let sessionSig = "";
  let lastPrs = 0;
  const sessions = gate(
    async () => {
      const list = await ac("session.list", {});
      const sig = Array.isArray(list)
        ? list.map((s) => `${s.id}:${s.running ? 1 : 0}:${s.meta?.worktree ?? ""}`).join("|")
        : "";
      if (sig !== sessionSig) {
        sessionSig = sig;
        overview();
      }
    },
    { debounceMs: 100 },
  );
  const overview = gate(() => ac("worktree.overview", {}), { debounceMs: 500 });
  const overlaps = gate(() => ac("dispatch.overlap_files", {}), {
    debounceMs: 300,
  });
  const wtStatus = gate(() => ac("worktree.status", { paths: allPaths }), {
    gapMs: 5000,
  });
  const prs = async () => {
    if (Date.now() - lastPrs < 180_000) return;
    lastPrs = Date.now();
    for (const p of projects) await ac("gh.list", { project: p.id, kind: "pr" });
  };
  const projectsLoad = gate(
    async () => {
      await ac("project.list", {});
      wtStatus();
      void prs();
    },
    { debounceMs: 100 },
  );
  const changes = gate(() => ac("git.changes", { worktree: selected }), {
    gapMs: 1000,
  });
  const usage = gate(
    async () => {
      await ac("agent.worktree_usage", { worktrees: allPaths });
      await ac("budget.status", { worktrees: allPaths });
    },
    { debounceMs: 400 },
  );
  const events = {};
  watch.onEvent = (e) => {
    if (stopping) return;
    const k = e.event ?? "?";
    events[k] = (events[k] ?? 0) + 1;
    if (k === "session.changed") sessions();
    if (k === "projects.changed") projectsLoad();
    if (["projects.changed", "dispatch.changed", "janitor.events", "pr.changed"].includes(k)) overview();
    if (["fs.changed", "projects.changed", "dispatch.changed"].includes(k)) overlaps();
    if (k === "fs.changed") changes();
    if (k === "pr.changed") {
      lastPrs = 0;
      void prs();
    }
    if (k === "usage.changed" || k === "budget.events") usage();
  };
  await watch.call("watch.subscribe", { worktree: selected });
  // first paint: every list once
  sessions();
  projectsLoad();
  overview();
  overlaps();
  changes();
  usage();
  ac("settings.get", {});
  const focus = () => {
    ac("project.list", {});
    ac("gh.pr_refresh", {});
    sessions();
  };
  timers.push(setInterval(focus, focusEvery));

  // ---- dispatch: a task worktree and a short worker in it ----
  let churned = 0;
  const shortWorker =
    process.platform === "win32" ? ["cmd.exe", ["/c", "ping -n 4 127.0.0.1 > nul"]] : ["sh", ["-c", "sleep 3"]];
  const churn = async () => {
    if (stopping) return;
    const p = projects[churned % nProjects];
    const name = `task${++churned}`;
    try {
      const wt = await app.call("worktree.create", {
        project: p.id,
        name,
        auto: true,
      });
      await app.call("session.spawn", {
        program: shortWorker[0],
        args: shortWorker[1],
        cwd: wt.path,
        cols: 80,
        rows: 24,
        label: name,
        meta: { kind: "agent", worktree: wt.path },
      });
    } catch (e) {
      console.error(`churn: ${e.message}`);
    }
  };
  if (churnEvery > 0) timers.push(setInterval(churn, churnEvery));

  // ---- the agents' own bookkeeping (hooks and CLI calls) ----
  const agentCalls = {};
  const tick = (a) => {
    if (stopping) return;
    const call = (m, p) => {
      agentCalls[m] = (agentCalls[m] ?? 0) + 1;
      return a.conn.call(m, p).catch(() => {});
    };
    a.n = (a.n ?? 0) + 1;
    call("session.hint", { id: a.id, state: "working" });
    setTimeout(() => !stopping && call("session.hint", { id: a.id, state: "idle" }), 2000);
    if (a.n % 3 === 0) call("msg.inbox", { session: a.id, mark: true });
    if (a.n % 4 === 1 && !a.main && a.mainId)
      call("msg.send", {
        from: a.id,
        to: a.mainId,
        text: `progress ${a.n}`,
        kind: "message",
        quiet: true,
      });
    if (a.n % 4 === 2) call("session.list", {});
    if (a.works)
      void (async () => {
        writeFileSync(join(a.cwd, `work-${a.id}.txt`), `turn ${a.n}\n`, {
          flag: "a",
        });
        await gitAsync(a.cwd, "status", "--porcelain");
        if (a.n % commitEvery === 0) {
          await gitAsync(a.cwd, "add", `work-${a.id}.txt`);
          await gitAsync(a.cwd, "commit", "-qm", `turn ${a.n} of agent ${a.id}`);
        }
      })().catch(() => {});
    timers.push(setTimeout(() => tick(a), agentEvery * (0.75 + Math.random() / 2)));
  };
  for (const a of agents) timers.push(setTimeout(() => tick(a), Math.random() * agentEvery));

  await sleep(warmupSecs * 1000);
  const sites = async () => {
    const st = await app.call("mem.status", {});
    const own = st?.breakdown?.own ?? {};
    const out = {};
    for (const s of own.sites ?? []) out[`${s.site}\t${s.command}`] = s.total;
    return out;
  };
  const snap = async () => ({
    load: await app.call("debug.load"),
    sites: await sites(),
    app: { ...appCalls },
    agent: { ...agentCalls },
    ev: { ...events },
  });
  const before = await snap();
  const t0 = Date.now();
  await sleep(idleSecs * 1000);
  const after = await snap();
  const mins = (Date.now() - t0) / 60000;
  const rate = (n) => +(n / mins).toFixed(1);
  const diff = (a, b) => {
    const out = {};
    for (const [k, v] of Object.entries(a)) {
      const d = v - (b[k] ?? 0);
      if (d > 0) out[k] = d;
    }
    return out;
  };

  const spawns = after.load.spawns - before.load.spawns;
  const bySite = Object.entries(diff(after.sites, before.sites))
    .map(([k, n]) => {
      const [site, command] = k.split("\t");
      return { site, command, count: n, per_min: rate(n) };
    })
    .sort((x, y) => y.count - x.count);
  const listed = bySite.reduce((s, r) => s + r.count, 0);
  if (spawns > listed)
    bySite.push({
      site: "(not in top 16)",
      command: "",
      count: spawns - listed,
      per_min: rate(spawns - listed),
    });
  const calls = {};
  for (const [m, v] of Object.entries(after.load.calls)) {
    const n = v.calls - (before.load.calls[m]?.calls ?? 0);
    if (n > 0 && m !== "debug.load" && m !== "mem.status")
      calls[m] = {
        calls: n,
        per_min: rate(n),
        avg_ms: Math.round((v.ms - (before.load.calls[m]?.ms ?? 0)) / n),
      };
  }
  report = {
    setup: {
      working: Number.isFinite(working) ? working : "all",
      projects: nProjects,
      worktrees: nWorktrees,
      agents: nAgents,
      terminals: nTerminals,
      remote: withRemote,
      warmup_secs: warmupSecs,
      idle_secs: idleSecs,
    },
    spawns: {
      total: spawns,
      per_min: rate(spawns),
      limit_per_min: Number.isFinite(maxSpawns) ? maxSpawns : null,
    },
    watches: after.load.watches,
    by_site: bySite,
    // what the set-up and the warm-up started (all time, before the window): the first looks
    warmup_by_site: before.sites,
    // children per call of a method, from the sites named after it: how much git one read costs (all worktrees or one)
    spawns_per_call: Object.fromEntries(
      Object.entries(calls)
        .map(([m, v]) => [m, bySite.filter((r) => r.site === m).reduce((n, r) => n + r.count, 0) / v.calls])
        .filter(([, n]) => n > 0)
        .map(([m, n]) => [m, +n.toFixed(1)]),
    ),
    calls_per_min: Object.fromEntries(Object.entries(calls).map(([m, v]) => [m, v.per_min])),
    calls,
    events_per_min: Object.fromEntries(Object.entries(diff(after.ev, before.ev)).map(([k, n]) => [k, rate(n)])),
  };
  if (report.spawns.per_min > maxSpawns) failed = new Error(`spawns ${report.spawns.per_min}/min > ${maxSpawns}`);
} catch (e) {
  failed = e;
} finally {
  stopping = true;
  for (const t of timers) clearTimeout(t);
  for (const c of conns) c.close();
  if (daemon && daemon.gone === undefined) daemon.kill();
  await sleep(500);
  try {
    rmSync(dir, { recursive: true, force: true });
  } catch {}
}

if (report) {
  if (asJson) console.log(JSON.stringify(report, null, 2));
  else {
    const s = report.setup;
    console.log(
      `idle-spawns: ${s.projects} projects, ${s.worktrees} worktrees, ${s.agents} agents, ${s.terminals} terminals, ${s.working} working, ${s.idle_secs}s window after ${s.warmup_secs}s warm-up`,
    );
    console.log(
      `  child processes: ${report.spawns.total} (${report.spawns.per_min}/min), watched dirs: ${report.watches}`,
    );
    console.log("  per min  count  site / command");
    for (const r of report.by_site)
      console.log(`  ${String(r.per_min).padStart(7)} ${String(r.count).padStart(6)}  ${r.site}  ${r.command}`);
    console.log("  calls per min (app + agents + daemon):");
    for (const [m, v] of Object.entries(report.calls).sort((a, b) => b[1].calls - a[1].calls))
      console.log(
        `  ${String(v.per_min).padStart(7)}  ${m} (avg ${v.avg_ms} ms${report.spawns_per_call[m] ? `, ${report.spawns_per_call[m]} children per call` : ""})`,
      );
    console.log(`  events per min: ${JSON.stringify(report.events_per_min)}`);
    console.log("  before the window (set-up and warm-up):");
    for (const [k, n] of Object.entries(report.warmup_by_site).sort((a, b) => b[1] - a[1]))
      console.log(`  ${String(n).padStart(7)}  ${k.replace("	", "  ")}`);
  }
}
if (failed) {
  console.error(`idle-spawns FAILED: ${failed.message}`);
  process.exit(1);
}
console.log("idle-spawns passed");
process.exit(0);
