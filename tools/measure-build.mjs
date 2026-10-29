// Wall time and peak memory of the Rust build steps, so a change to profiles, linker or jobs can be judged by numbers.
//
//   node tools/measure-build.mjs [--label NAME] [--jobs 2,default] [--steps check,test-build,relink]
//                                [--target-dir DIR] [--keep] [--json FILE]
//
//   check       cargo check --workspace                                   (from an empty target dir)
//   test-build  cargo test --no-run -p axelstra-core -p axelstra-daemon -p axelstra   (from an empty target dir, includes the links)
//   relink      touch crates/cli/src/main.rs, then cargo test --no-run again (one crate compiled and every test binary re-linked
//               that depends on it; run after test-build in the same target dir)
//
// Each step runs once per `--jobs` value, each in a fresh target dir under the temp dir (`--target-dir` picks the parent),
// so baseline and after runs are comparable: use the same --label scheme and compare the JSON. "default" leaves
// CARGO_BUILD_JOBS unset (cargo uses every core). Peak memory is the highest sum of working set / RSS over the whole process
// tree of the cargo command, sampled every 500 ms; a short-lived spike between two samples can be missed, so read it as a floor.
// Heavy: run it inside the lock, one at a time: `axelstra lock verify -- node tools/measure-build.mjs ...`.
import { spawn, spawnSync } from "node:child_process";
import { mkdirSync, rmSync, utimesSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const argv = process.argv.slice(2);
const opt = (name, dflt) => {
  const i = argv.indexOf(name);
  return i >= 0 ? argv[i + 1] : dflt;
};
const label = opt("--label", "run");
const jobsList = opt("--jobs", "2,default").split(",");
const stepNames = opt("--steps", "check,test-build,relink").split(",");
const parent = opt("--target-dir", join(tmpdir(), "axelstra-measure"));
const jsonOut = opt("--json", null);
const keep = argv.includes("--keep");

const TEST_PKGS = ["-p", "axelstra-core", "-p", "axelstra-daemon", "-p", "axelstra"];
const STEPS = {
  check: { args: ["check", "--workspace"], fresh: true },
  "test-build": { args: ["test", "--no-run", ...TEST_PKGS], fresh: true },
  relink: { args: ["test", "--no-run", ...TEST_PKGS], fresh: false, touch: "crates/cli/src/main.rs" },
};

// Peak of the process tree rooted at `pid`. One long-lived sampler process, not one per poll.
function startSampler(pid) {
  let peak = 0;
  let child;
  if (process.platform === "win32") {
    const ps = `
$root = ${pid}
while ($true) {
  $all = Get-CimInstance Win32_Process -Property ProcessId, ParentProcessId, WorkingSetSize
  $kids = $all | Group-Object ParentProcessId -AsHashTable -AsString
  $stack = [System.Collections.Generic.Stack[uint32]]::new(); $stack.Push($root)
  $sum = [uint64]0
  while ($stack.Count) { $p = $stack.Pop(); foreach ($c in $kids["$p"]) { if ($c) { $sum += $c.WorkingSetSize; $stack.Push($c.ProcessId) } } }
  Write-Output $sum
  Start-Sleep -Milliseconds 500
}`;
    child = spawn("powershell", ["-NoProfile", "-NonInteractive", "-Command", ps], {
      stdio: ["ignore", "pipe", "ignore"],
    });
  } else {
    const sh = `while :; do ps -eo pid=,ppid=,rss= | awk -v r=${pid} '{p[$1]=$2; m[$1]=$3} END {s=0; for (i in p) {j=i; while (j!="" && j!=0) {if (j==r) {s+=m[i]; break} j=p[j]}} print s*1024}'; sleep 0.5; done`;
    child = spawn("sh", ["-c", sh], { stdio: ["ignore", "pipe", "ignore"] });
  }
  child.stdout.on("data", (d) => {
    for (const line of String(d).split(/\r?\n/)) {
      const n = Number(line);
      if (n > peak) peak = n;
    }
  });
  return {
    stop() {
      child.kill();
      if (process.platform === "win32")
        spawnSync("taskkill", ["/pid", String(child.pid), "/t", "/f"], { stdio: "ignore" });
      return peak;
    },
  };
}

function runCargo(args, env, cwd) {
  return new Promise((resolve) => {
    const t0 = Date.now();
    const c = spawn("cargo", args, { cwd, env, stdio: ["ignore", "ignore", "pipe"] });
    const tail = [];
    c.stderr.on("data", (d) => {
      tail.push(String(d));
      if (tail.length > 20) tail.shift();
    });
    const sampler = startSampler(c.pid);
    c.on("close", (code) => {
      resolve({ code, secs: (Date.now() - t0) / 1000, peakBytes: sampler.stop(), tail: tail.join("").slice(-600) });
    });
  });
}

function dirSize(dir) {
  const r = spawnSync(
    process.platform === "win32" ? "powershell" : "du",
    process.platform === "win32"
      ? [
          "-NoProfile",
          "-Command",
          `(Get-ChildItem -LiteralPath '${dir}' -Recurse -File -ErrorAction SilentlyContinue | Measure-Object Length -Sum).Sum`,
        ]
      : ["-sb", dir],
    { encoding: "utf8" },
  );
  return Number(String(r.stdout).trim().split(/\s+/)[0]) || 0;
}

const gb = (b) => (b / 1024 ** 3).toFixed(2);
const rows = [];
mkdirSync(parent, { recursive: true });
for (const jobs of jobsList) {
  const targetDir = join(parent, `${label}-j${jobs}`);
  rmSync(targetDir, { recursive: true, force: true });
  for (const name of stepNames) {
    const step = STEPS[name];
    if (!step) throw new Error(`unknown step ${name}; known: ${Object.keys(STEPS).join(", ")}`);
    const env = {
      ...process.env,
      CARGO_TARGET_DIR: targetDir,
      CARGO_INCREMENTAL: process.env.CARGO_INCREMENTAL ?? "1",
    };
    if (jobs === "default") delete env.CARGO_BUILD_JOBS;
    else env.CARGO_BUILD_JOBS = jobs;
    if (step.fresh && name !== stepNames[0]) rmSync(targetDir, { recursive: true, force: true });
    if (step.touch) {
      const f = join(root, step.touch);
      const now = new Date();
      utimesSync(f, now, now);
    }
    console.error(`[${label}] jobs=${jobs} step=${name} ...`);
    const r = await runCargo(step.args, env, root);
    const row = {
      label,
      jobs,
      step: name,
      ok: r.code === 0,
      secs: Math.round(r.secs * 10) / 10,
      peakGB: Number(gb(r.peakBytes)),
      targetGB: Number(gb(dirSize(targetDir))),
    };
    if (r.code !== 0) row.error = r.tail;
    rows.push(row);
    console.error(JSON.stringify(row));
  }
  if (!keep) rmSync(targetDir, { recursive: true, force: true });
}

console.log("\n| label | jobs | step | ok | seconds | peak GB (tree) | target GB |\n|---|---|---|---|---|---|---|");
for (const r of rows)
  console.log(
    `| ${r.label} | ${r.jobs} | ${r.step} | ${r.ok ? "yes" : "NO"} | ${r.secs} | ${r.peakGB} | ${r.targetGB} |`,
  );
if (jsonOut) writeFileSync(jsonOut, JSON.stringify(rows, null, 2));
