// What a branch touched, for `node scripts/check.mjs --fast`. The decisions are pure functions so they can be tested
// without git or cargo; `gitChanged` and `workspacePackages` are the thin wrappers that read the real repo.
import { spawnSync } from "node:child_process";
import { relative, resolve, sep } from "node:path";

// A change to any of these can alter how every crate builds or lints, so every package is affected.
const GLOBAL = [
  /^Cargo\.(toml|lock)$/,
  /^rust-toolchain\.toml$/,
  /^rustfmt\.toml$/,
  /^clippy\.toml$/,
  /^deny\.toml$/,
  /^\.cargo\//,
];

// files: repo-relative, forward slashes. packages: [{name, dir (repo-relative, no trailing slash), deps: [names]}].
// Returns the packages that must be re-checked: the touched ones and everything that depends on them.
export function affectedPackages(files, packages) {
  if (files.some((f) => GLOBAL.some((g) => g.test(f)))) return packages.map((p) => p.name).sort();
  const hit = new Set();
  for (const f of files) {
    // longest directory wins: app/src-tauri is inside app/, which is not a package
    const owner = packages
      .filter((p) => f === p.dir || f.startsWith(`${p.dir}/`))
      .sort((a, b) => b.dir.length - a.dir.length)[0];
    if (owner) hit.add(owner.name);
  }
  let grew = true;
  while (grew) {
    grew = false;
    for (const p of packages) {
      if (!hit.has(p.name) && p.deps.some((d) => hit.has(d))) {
        hit.add(p.name);
        grew = true;
      }
    }
  }
  return [...hit].sort();
}

// The frontend files eslint should look at (ts/tsx under app/src), relative to app/.
export function lintTargets(files) {
  return files.filter((f) => /^app\/src\/.*\.(ts|tsx)$/.test(f)).map((f) => f.slice("app/".length));
}

export function touchesFrontend(files) {
  return files.some((f) => f.startsWith("app/") && !f.startsWith("app/src-tauri/"));
}

export function touchesWorkflows(files) {
  return files.some((f) => f.startsWith(".github/workflows/"));
}

const git = (root, ...a) => spawnSync("git", a, { cwd: root, encoding: "utf8", maxBuffer: 64 << 20 });

// The merge-base with the default branch (origin/main, else main), or null when there is none.
export function baseCommit(root) {
  for (const ref of ["origin/main", "main"]) {
    if (git(root, "rev-parse", "--verify", "-q", ref).status !== 0) continue;
    const mb = git(root, "merge-base", "HEAD", ref).stdout.trim();
    if (mb) return mb;
  }
  return null;
}

// Files changed since `base` (committed or not) plus untracked ones, repo-relative with forward slashes.
export function gitChanged(root, base) {
  const out = [
    ...git(root, "diff", "--name-only", base).stdout.split("\n"),
    ...git(root, "ls-files", "--others", "--exclude-standard").stdout.split("\n"),
  ];
  return [...new Set(out.map((f) => f.trim().replaceAll("\\", "/")).filter(Boolean))];
}

// Workspace members and their path dependencies, from `cargo metadata --no-deps` (reads manifests, compiles nothing).
export function workspacePackages(root) {
  const r = spawnSync("cargo", ["metadata", "--no-deps", "--format-version", "1", "--offline"], {
    cwd: root,
    encoding: "utf8",
    maxBuffer: 64 << 20,
  });
  if (r.status !== 0) return null;
  const meta = JSON.parse(r.stdout);
  const names = new Set(meta.packages.map((p) => p.name));
  return meta.packages.map((p) => ({
    name: p.name,
    dir: relative(root, resolve(p.manifest_path, "..")).split(sep).join("/"),
    deps: p.dependencies.map((d) => d.name).filter((n) => names.has(n)),
  }));
}

export function selfTest() {
  const eq = (got, want, msg) => {
    if (JSON.stringify(got) !== JSON.stringify(want))
      throw new Error(`self-test failed: ${msg}: got ${JSON.stringify(got)}, want ${JSON.stringify(want)}`);
  };
  const pk = [
    { name: "axelstra-core", dir: "crates/core", deps: [] },
    { name: "axelstra-daemon", dir: "crates/daemon", deps: ["axelstra-core"] },
    { name: "axelstra", dir: "crates/cli", deps: ["axelstra-core", "axelstra-daemon"] },
    { name: "axelstra-app", dir: "app/src-tauri", deps: ["axelstra-core"] },
  ];
  eq(affectedPackages(["crates/cli/src/main.rs"], pk), ["axelstra"], "leaf crate only");
  eq(
    affectedPackages(["crates/core/src/msg.rs"], pk),
    ["axelstra", "axelstra-app", "axelstra-core", "axelstra-daemon"],
    "core drags its dependents",
  );
  eq(affectedPackages(["crates/daemon/src/x.rs"], pk), ["axelstra", "axelstra-daemon"], "daemon drags cli");
  eq(affectedPackages(["app/src-tauri/src/main.rs"], pk), ["axelstra-app"], "longest dir wins");
  eq(affectedPackages(["app/src/store.ts", "docs/TOOLS.md"], pk), [], "no rust files: nothing");
  eq(affectedPackages(["Cargo.lock"], pk).length, 4, "Cargo.lock: all");
  eq(affectedPackages([".cargo/config.toml"], pk).length, 4, ".cargo: all");
  eq(affectedPackages(["crates/corefoo/x.rs"], pk), [], "a sibling prefix is not the crate");
  eq(
    lintTargets(["app/src/a.ts", "app/src/b/c.tsx", "app/src/s.css", "app/vite.config.ts"]),
    ["src/a.ts", "src/b/c.tsx"],
    "lint targets",
  );
  eq(touchesFrontend(["app/src-tauri/src/main.rs"]), false, "tauri host is not frontend");
  eq(touchesFrontend(["app/package.json"]), true, "package.json is frontend");
  eq(touchesWorkflows([".github/workflows/ci.yml"]), true, "workflows");
  console.log("changed self-test ok");
}
