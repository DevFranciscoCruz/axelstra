// Sets one version in all three places and refreshes Cargo.lock offline.
//
//   node scripts/bump-version.mjs 0.2.0       (or v0.2.0)
//   node scripts/bump-version.mjs --no-lock   skip the Cargo.lock refresh
//
// Uses the same fields and rewrite rules as stamp-version.mjs, so what this writes passes
// `stamp-version.mjs --check`. Releases still go by tag; this is for bumping the tree on purpose.
import { spawnSync } from "node:child_process";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { stamp, check } from "./stamp-version.mjs";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const args = process.argv.slice(2);
const noLock = args.includes("--no-lock");
const arg = args.find((a) => !a.startsWith("--"));
if (!arg) {
  console.error("usage: node scripts/bump-version.mjs <X.Y.Z> [--no-lock]");
  process.exit(2);
}
const tag = arg.startsWith("v") ? arg : `v${arg}`;
let version;
try {
  version = stamp(root, tag);
  check(root);
} catch (e) {
  console.error(e.message);
  process.exit(1);
}
console.log(`version fields set to ${version}`);
if (!noLock) {
  // --workspace updates only the workspace members' own versions in Cargo.lock; no third-party pin moves
  const r = spawnSync("cargo", ["update", "--workspace", "--offline"], { cwd: root, stdio: "inherit" });
  if (r.status !== 0) {
    console.error("Cargo.lock refresh failed; run `cargo update --workspace --offline` by hand");
    process.exit(1);
  }
  console.log("Cargo.lock refreshed");
}
console.log("review with `git diff`, then commit the three files and Cargo.lock");
