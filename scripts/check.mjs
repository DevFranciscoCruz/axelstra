// One command for the whole local gate: fmt --check, clippy, cargo test, tsc, eslint, prettier (changed files), vitest.
//
//   node scripts/check.mjs                 everything
//   node scripts/check.mjs --skip-rust     only the frontend steps
//   node scripts/check.mjs --skip-frontend only the Rust steps
//   --skip-fmt --skip-clippy --skip-test --skip-tsc --skip-lint --skip-format --skip-vitest   skip one step
//   --engine                               Rust steps only for core, daemon and cli (no desktop toolkit needed)
//
// Steps run in order and the first failure stops the run. Inside an Axelstra worktree, run it through
// `axelstra lock verify -- node scripts/check.mjs` so heavy builds do not pile up.
import { spawnSync } from "node:child_process";
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
]);
for (const f of flags) {
  if (!known.has(f)) {
    console.error(`unknown flag ${f}; known: ${[...known].join(" ")}`);
    process.exit(2);
  }
}
const engine = flags.has("--engine");
const pkgs = engine ? ["-p", "axelstra-core", "-p", "axelstra-daemon", "-p", "axelstra"] : null;
const npx = process.platform === "win32" ? "npx.cmd" : "npx";
const npm = process.platform === "win32" ? "npm.cmd" : "npm";

const steps = [
  { name: "fmt", skip: "--skip-rust", also: "--skip-fmt", cmd: ["cargo", "fmt", "--all", "--check"] },
  {
    name: "clippy",
    skip: "--skip-rust",
    also: "--skip-clippy",
    cmd: ["cargo", "clippy", ...(pkgs ?? ["--workspace"]), "--all-targets", "--", "-D", "warnings"],
  },
  {
    name: "test",
    skip: "--skip-rust",
    also: "--skip-test",
    cmd: [
      "cargo",
      "test",
      "--no-fail-fast",
      ...(pkgs ?? ["-p", "axelstra-core", "-p", "axelstra-daemon", "-p", "axelstra", "-p", "axelstra-app"]),
    ],
  },
  { name: "tsc", skip: "--skip-frontend", also: "--skip-tsc", cmd: [npx, "tsc", "--noEmit"], cwd: join(root, "app") },
  { name: "lint", skip: "--skip-frontend", also: "--skip-lint", cmd: [npm, "run", "lint"], cwd: join(root, "app") },
  {
    name: "format",
    skip: "--skip-frontend",
    also: "--skip-format",
    cmd: [process.execPath, "scripts/check-format.mjs"],
  },
  { name: "vitest", skip: "--skip-frontend", also: "--skip-vitest", cmd: [npm, "test"], cwd: join(root, "app") },
];

const results = [];
for (const s of steps) {
  if (flags.has(s.skip) || flags.has(s.also)) {
    results.push(`skip  ${s.name}`);
    continue;
  }
  console.log(`\n== ${s.name}: ${s.cmd.join(" ")}`);
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
}
console.log(`\n${results.join("\n")}`);
