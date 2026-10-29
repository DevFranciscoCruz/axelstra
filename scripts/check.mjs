// One command for the whole local gate: fmt --check, clippy, cargo test, tsc, eslint, prettier (changed files), vitest.
//
//   node scripts/check.mjs                 everything
//   node scripts/check.mjs --skip-rust     only the frontend steps
//   node scripts/check.mjs --skip-frontend only the Rust steps
//   --skip-fmt --skip-clippy --skip-test --skip-tsc --skip-lint --skip-format --skip-vitest   skip one step
//   --skip-workflows                       (actionlint on .github/workflows; skipped with a notice when not on PATH)
//   --engine                             Rust steps only for core, daemon and cli (no desktop toolkit needed)
//
// Steps run in order and the first failure stops the run. Inside an Axelstra worktree, run it through
// `axelstra lock verify -- node scripts/check.mjs` so heavy builds do not pile up.
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, readFileSync, readdirSync } from "node:fs";
import { homedir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const flags = new Set(process.argv.slice(2));
const known = new Set([
  "--engine",
  "--skip-rust",
  "--skip-frontend",
  "--skip-fmt",
  "--skip-clippy",
  "--skip-test",
  "--skip-tsc",
  "--skip-lint",
  "--skip-format",
  "--skip-vitest",
  "--skip-workflows",
]);
for (const f of flags) {
  if (!known.has(f)) {
    console.error(`unknown flag ${f}; known: ${[...known].join(" ")}`);
    process.exit(2);
  }
}
const engine = flags.has("--engine");
const pkgs = engine
  ? ["-p", "axelstra-core", "-p", "axelstra-daemon", "-p", "axelstra"]
  : null;
const npx = process.platform === "win32" ? "npx.cmd" : "npx";
const npm = process.platform === "win32" ? "npm.cmd" : "npm";

const steps = [
  {
    name: "fmt",
    skip: "--skip-rust",
    also: "--skip-fmt",
    cmd: ["cargo", "fmt", "--all", "--check"],
  },
  {
    name: "clippy",
    skip: "--skip-rust",
    also: "--skip-clippy",
    cmd: [
      "cargo",
      "clippy",
      ...(pkgs ?? ["--workspace"]),
      "--all-targets",
      "--",
      "-D",
      "warnings",
    ],
  },
  {
    name: "test",
    skip: "--skip-rust",
    also: "--skip-test",
    cmd: [
      "cargo",
      "test",
      "--no-fail-fast",
      ...(pkgs ?? [
        "-p",
        "axelstra-core",
        "-p",
        "axelstra-daemon",
        "-p",
        "axelstra",
        "-p",
        "axelstra-app",
      ]),
    ],
  },
  {
    name: "tsc",
    skip: "--skip-frontend",
    also: "--skip-tsc",
    cmd: [npx, "tsc", "--noEmit"],
    cwd: join(root, "app"),
  },
  {
    name: "lint",
    skip: "--skip-frontend",
    also: "--skip-lint",
    cmd: [npm, "run", "lint"],
    cwd: join(root, "app"),
  },
  {
    name: "format",
    skip: "--skip-frontend",
    also: "--skip-format",
    cmd: [process.execPath, "scripts/check-format.mjs"],
  },
  {
    name: "i18n",
    skip: "--skip-frontend",
    also: "--skip-lint",
    cmd: [process.execPath, "scripts/check-i18n-static.mjs"],
  },
  {
    name: "vitest",
    skip: "--skip-frontend",
    also: "--skip-vitest",
    cmd: [npm, "test"],
    cwd: join(root, "app"),
  },
  // actionlint is a Go binary, not an npm dependency: run it when it is on PATH, else say so and move on
  {
    name: "workflows",
    skip: "--skip-workflows",
    also: "--skip-workflows",
    cmd: ["actionlint"],
    optional: true,
  },
];

function onPath(cmd) {
  const r = spawnSync(cmd, ["-version"], { stdio: "ignore" });
  return !r.error && r.status === 0;
}

// Guard for docs/TESTING.md: a test run must leave the user's real data dir alone. Hashes only the files a test could
// change; a running real daemon rewrites sessions.json, lag.log and friends on its own, so those are not compared.
function realDataDir() {
  if (process.platform === "win32")
    return process.env.LOCALAPPDATA
      ? join(process.env.LOCALAPPDATA, "axelstra")
      : null;
  if (process.platform === "darwin")
    return join(homedir(), "Library", "Application Support", "axelstra");
  return join(
    process.env.XDG_DATA_HOME || join(homedir(), ".local", "share"),
    "axelstra",
  );
}
const GUARDED = ["hooks.json", "token", "projects.json", "settings.json"];
// settings.json also holds the UI choices (`ui`) that the running app and daemon rewrite
// on their own (selected worktree, ...), so a live app would trip the guard: hash it without them.
function fileHash(f, p) {
  let buf = readFileSync(p);
  if (f === "settings.json") {
    try {
      const j = JSON.parse(buf.toString("utf8"));
      const d = j && typeof j === "object" && j.data ? j.data : j;
      if (d && typeof d === "object") delete d.ui;
      buf = Buffer.from(JSON.stringify(j));
    } catch {}
  }
  return createHash("sha256").update(buf).digest("hex");
}

function snapshot() {
  const dir = realDataDir();
  if (!dir || !existsSync(dir)) return null;
  const snap = {};
  for (const f of GUARDED) {
    const p = join(dir, f);
    snap[f] = existsSync(p) ? fileHash(f, p) : "(absent)";
  }
  return snap;
}

const results = [];
for (const s of steps) {
  if (flags.has(s.skip) || flags.has(s.also)) {
    results.push(`skip  ${s.name}`);
    continue;
  }
  if (s.optional && !onPath(s.cmd[0])) {
    results.push(
      `skip  ${s.name} (${s.cmd[0]} not on PATH; https://github.com/rhysd/actionlint)`,
    );
    continue;
  }
  console.log(`\n== ${s.name}: ${s.cmd.join(" ")}`);
  const before = s.name === "test" ? snapshot() : null;
  const t0 = Date.now();
  const r = spawnSync(s.cmd[0], s.cmd.slice(1), {
    cwd: s.cwd ?? root,
    stdio: "inherit",
    shell: s.cmd[0].endsWith(".cmd"),
  });
  const secs = ((Date.now() - t0) / 1000).toFixed(1);
  if (r.error || r.status !== 0) {
    results.push(`FAIL  ${s.name} (${secs}s)`);
    console.error(`\n${results.join("\n")}`);
    process.exit(r.status || 1);
  }
  results.push(`ok    ${s.name} (${secs}s)`);
  if (s.name === "test") {
    const after = snapshot();
    if (!before) {
      results.push("skip  real-dir guard (no real data dir)");
    } else {
      const changed = Object.keys(before).filter(
        (k) => before[k] !== after?.[k],
      );
      if (changed.length) {
        results.push(
          `FAIL  real-dir guard: the test run changed ${changed.join(", ")}`,
        );
        console.error(`\n${results.join("\n")}`);
        process.exit(1);
      }
      results.push(
        "ok    real-dir guard (hooks.json, token, projects.json, settings.json unchanged)",
      );
    }
  }
}
console.log(`\n${results.join("\n")}`);
