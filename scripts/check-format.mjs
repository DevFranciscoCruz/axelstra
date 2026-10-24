// Prettier check that does not reformat the repo: the tree was never auto-formatted, and a mass reformat
// would bury real diffs.
//
//   node scripts/check-format.mjs [--base <ref>]     default base: origin/main, then main
//   node scripts/check-format.mjs --all              every tracked file (shows how far the repo is from formatted)
//
// Rules, against the merge-base with the base ref:
//   - a file added since then (or untracked) must be formatted;
//   - a file changed since then must be formatted, unless it was already unformatted at the base
//     (touching a legacy file never forces a whole-file reformat).
// Only app/src, scripts/*.mjs and tools/*.mjs are checked, with app/.prettierrc.json.
import { spawnSync } from "node:child_process";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const args = process.argv.slice(2);
const all = args.includes("--all");
const bi = args.indexOf("--base");
const git = (...a) => spawnSync("git", a, { cwd: root, encoding: "utf8", maxBuffer: 64 << 20 });
const base =
  bi >= 0 ? args[bi + 1] : ["origin/main", "main"].find((r) => git("rev-parse", "--verify", "-q", r).status === 0);
const wanted = /^(app\/src\/.*\.(ts|tsx|css)|scripts\/.*\.mjs|tools\/.*\.mjs)$/;
const cfg = join(root, "app", ".prettierrc.json");
const bin = join(root, "app", "node_modules", "prettier", "bin", "prettier.cjs");
const prettier = (extra, input) =>
  spawnSync(process.execPath, [bin, "--config", cfg, ...extra], { cwd: root, encoding: "utf8", input });
const norm = (list) =>
  [...new Set(list.split("\n").map((f) => f.trim().replaceAll("\\", "/")))].filter((f) => wanted.test(f));

const mb = !all && base ? git("merge-base", "HEAD", base).stdout.trim() : "";
let added, changed;
if (all) {
  added = norm(git("ls-files").stdout);
  changed = [];
} else if (!mb) {
  console.log("format: no base ref found, checking nothing (pass --base <ref>)");
  process.exit(0);
} else {
  const status = git("diff", "--name-status", "--diff-filter=AM", mb).stdout.split("\n");
  added = norm(
    [
      ...status.filter((l) => l.startsWith("A")).map((l) => l.slice(1)),
      ...git("ls-files", "--others", "--exclude-standard").stdout.split("\n"),
    ].join("\n"),
  );
  changed = norm(
    status
      .filter((l) => l.startsWith("M"))
      .map((l) => l.slice(1))
      .join("\n"),
  );
}
// a modified file already unformatted at the base is legacy: leave it alone
const legacy = (f) => {
  const old = git("show", `${mb}:${f}`);
  return old.status === 0 && prettier(["--stdin-filepath", f, "--check"], old.stdout).status !== 0;
};
const files = [...added, ...changed.filter((f) => !legacy(f))];
if (!files.length) {
  console.log("format: no changed files to check");
  process.exit(0);
}
const r = prettier(["--check", ...files]);
process.stdout.write(r.stdout ?? "");
process.stderr.write(r.stderr ?? "");
process.exit(r.status ?? 1);
