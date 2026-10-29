// One command for the whole local gate: fmt --check, clippy, cargo test, tsc, eslint, prettier (changed files), vitest.
//
//   node scripts/check.mjs                 everything
//   node scripts/check.mjs --skip-rust     only the frontend steps
//   node scripts/check.mjs --skip-frontend only the Rust steps
//   --skip-fmt --skip-clippy --skip-test --skip-tsc --skip-lint --skip-format --skip-vitest   skip one step
//   --skip-workflows                       (actionlint on .github/workflows; skipped with a notice when not on PATH)
//   --engine                             Rust steps only for core, daemon and cli (no desktop toolkit needed)
//   --fast                                 quick loop while working, NOT the gate (CI and the push gate run the full mode):
//                                          clippy (which type-checks the tests, nothing is linked or run) only on the packages
//                                          this branch touched plus their dependents, no cargo test; tsc, eslint (changed files)
//                                          and vitest (--changed) only when app/ changed; actionlint only when workflows changed
//   --self-test                            the pure helpers (jobs for this machine, affected packages), no compile
//
// CARGO_BUILD_JOBS: when it is not set (and CI is not), it is derived from the cores and the free memory
// (scripts/lib/jobs.mjs) so a small machine does not run out of RAM; an explicit value always wins.
//
// Steps run in order and the first failure stops the run. Inside an Axelstra worktree, run it through
// `axelstra lock verify -- node scripts/check.mjs` so heavy builds do not pile up.
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, readFileSync, readdirSync } from "node:fs";
import { homedir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import * as diffs from "./lib/changed.mjs";
import * as jobs from "./lib/jobs.mjs";

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
  "--fast",
  "--self-test",
]);
for (const f of flags) {
  if (!known.has(f)) {
    console.error(`unknown flag ${f}; known: ${[...known].join(" ")}`);
    process.exit(2);
  }
}
if (flags.has("--self-test")) {
  jobs.selfTest();
  diffs.selfTest();
  process.exit(0);
}
const engine = flags.has("--engine");
const fast = flags.has("--fast");
// CI machines and explicit settings are left alone; a local run adapts to the machine
if (!process.env.CI && !process.env.CARGO_BUILD_JOBS) {
  process.env.CARGO_BUILD_JOBS = String(jobs.defaultJobs());
  console.log(
    `CARGO_BUILD_JOBS=${process.env.CARGO_BUILD_JOBS} (derived from cores and free memory; set it to override)`,
  );
}
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

// --fast: narrow the steps to what this branch touched. Returns a step, or {skipNote} to skip it.
function narrow(s, ctx) {
  const rust = ctx.pkgs;
  switch (s.name) {
    case "clippy": {
      const hit = engine ? rust.filter((p) => p !== "axelstra-app") : rust;
      if (!hit.length) return { skipNote: "no Rust package touched" };
      return {
        ...s,
        cmd: [
          "cargo",
          "clippy",
          ...hit.flatMap((p) => ["-p", p]),
          "--all-targets",
          "--",
          "-D",
          "warnings",
        ],
      };
    }
    case "test":
      return {
        skipNote:
          "--fast: clippy --all-targets already type-checks the tests; the full mode runs them",
      };
    case "tsc":
    case "vitest": {
      if (!diffs.touchesFrontend(ctx.files))
        return { skipNote: "app/ not touched" };
      return s.name === "vitest"
        ? { ...s, cmd: [npx, "vitest", "run", "--changed", ctx.base] }
        : s;
    }
    case "lint": {
      const t = diffs.lintTargets(ctx.files);
      return t.length
        ? { ...s, cmd: [npx, "eslint", ...t] }
        : { skipNote: "no app/src ts file touched" };
    }
    case "workflows":
      return diffs.touchesWorkflows(ctx.files)
        ? s
        : { skipNote: "workflows not touched" };
    default:
      return s; // fmt, format and i18n are cheap and already look at the whole tree or only changed files
  }
}

let fastCtx = null;
if (fast) {
  const base = diffs.baseCommit(root);
  const pk = diffs.workspacePackages(root);
  if (!base || !pk) {
    console.error(
      "--fast needs git with origin/main or main, and cargo metadata; run the full mode instead",
    );
    process.exit(2);
  }
  const files = diffs.gitChanged(root, base);
  fastCtx = { base, files, pkgs: diffs.affectedPackages(files, pk) };
  console.log(
    `fast: ${files.length} changed file(s) since ${base.slice(0, 7)}; Rust packages: ${fastCtx.pkgs.join(", ") || "none"}`,
  );
}

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
function snapshot() {
  const dir = realDataDir();
  if (!dir || !existsSync(dir)) return null;
  const snap = {};
  for (const f of GUARDED) {
    const p = join(dir, f);
    snap[f] = existsSync(p)
      ? createHash("sha256").update(readFileSync(p)).digest("hex")
      : "(absent)";
  }
  return snap;
}

const results = [];
for (const step of steps) {
  let s = step;
  if (fastCtx && !flags.has(s.skip) && !flags.has(s.also)) {
    s = narrow(step, fastCtx);
    if (s.skipNote) {
      results.push(`skip  ${step.name} (${s.skipNote})`);
      continue;
    }
  }
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
