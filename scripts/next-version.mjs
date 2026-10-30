// The version the next release gets, from the Conventional Commits since the last `vX.Y.Z` tag.
//
//   node scripts/next-version.mjs                 prints the next version (the last tag's when nothing is releasable)
//   node scripts/next-version.mjs --json          {tag, base, bump, next, counts}
//   node scripts/next-version.mjs --local         a version for a local build: the next one, or base + patch when
//                                                 nothing is releasable
//   --above X.Y.Z                                 raise the result above X.Y.Z (the installed copy), so the installer
//                                                 replaces it: X.Y.(Z+1) when the result is not already higher
//   --self-test                                   the pure helpers, no git
//   --root <dir>                                  repo root (default: the parent of scripts/)
//
// Same rules as release-please (release-please-config.json), so a local build and the release PR agree:
// a breaking change (`type!:` or a `BREAKING CHANGE:` footer) is a major bump, a minor one while the major is 0
// (`bump-minor-pre-major`); `feat` is minor; every other type shown in the changelog (`changelog-sections` without
// `hidden`) is a patch; hidden types (docs, test, ci, chore...) and non-conventional subjects release nothing.
// Merge commits are skipped.
import { spawnSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { check } from "./stamp-version.mjs";

const SUBJECT_RE = /^(\w+)(?:\([^)]*\))?(!)?: \S/;
const BREAKING_RE = /^BREAKING[ -]CHANGE: /m;
const RANK = { none: 0, patch: 1, minor: 2, major: 3 };

export function parseVersion(v) {
  const m = /^v?(\d+)\.(\d+)\.(\d+)$/.exec(v.trim());
  if (!m) throw new Error(`bad version ${v}: expected X.Y.Z`);
  return m.slice(1, 4).map(Number);
}

export function compare(a, b) {
  const [x, y] = [parseVersion(a), parseVersion(b)];
  for (let i = 0; i < 3; i++) if (x[i] !== y[i]) return x[i] < y[i] ? -1 : 1;
  return 0;
}

export function bumpOf(commit, releasable) {
  const m = SUBJECT_RE.exec(commit.subject);
  if (!m) return "none";
  if (m[2] || BREAKING_RE.test(commit.body ?? "")) return "major";
  if (m[1] === "feat") return "minor";
  return releasable.has(m[1]) ? "patch" : "none";
}

export function apply(base, bump) {
  const [major, minor, patch] = parseVersion(base);
  // pre-1.0, a breaking change is a minor bump (1.0.0 is the owner's decision, cut with Release-As)
  if (bump === "major" && major === 0) bump = "minor";
  if (bump === "major") return `${major + 1}.0.0`;
  if (bump === "minor") return `${major}.${minor + 1}.0`;
  if (bump === "patch") return `${major}.${minor}.${patch + 1}`;
  return base;
}

export function plan(base, commits, releasable) {
  const counts = {};
  let bump = "none";
  for (const c of commits) {
    const b = bumpOf(c, releasable);
    const type = SUBJECT_RE.exec(c.subject)?.[1] ?? "other";
    counts[type] = (counts[type] ?? 0) + 1;
    if (RANK[b] > RANK[bump]) bump = b;
  }
  return { base, bump, next: bump === "none" ? null : apply(base, bump), counts };
}

// a version for a local installer: the next release, else base + patch, and always above what is installed
export function local(p, above) {
  let v = p.next ?? apply(p.base, "patch");
  if (above && compare(v, above) <= 0) v = apply(above.replace(/^v/, ""), "patch");
  return v;
}

export function releasableTypes(config) {
  const sections = config.packages?.["."]?.["changelog-sections"] ?? [];
  return new Set(sections.filter((s) => !s.hidden && s.type !== "feat").map((s) => s.type));
}

function git(root, args) {
  const r = spawnSync("git", args, { cwd: root, encoding: "utf8" });
  return r.status === 0 ? r.stdout : null;
}

function commitsSince(root, tag) {
  const range = tag ? [`${tag}..HEAD`] : ["HEAD"];
  const out = git(root, ["log", "--no-merges", "--format=%s%x1f%b%x1e", ...range]);
  if (out === null) throw new Error("git log failed");
  return out
    .split("\x1e")
    .map((r) => r.replace(/^\s+/, ""))
    .filter(Boolean)
    .map((r) => {
      const [subject, body] = r.split("\x1f");
      return { subject: subject.trim(), body: body ?? "" };
    });
}

export function compute(root) {
  const config = JSON.parse(readFileSync(join(root, "release-please-config.json"), "utf8"));
  const tag = git(root, ["describe", "--tags", "--abbrev=0", "--match", "v[0-9]*"])?.trim() || null;
  const base = tag ? tag.slice(1) : check(root);
  return { tag, ...plan(base, commitsSince(root, tag), releasableTypes(config)) };
}

function selfTest() {
  const assert = (cond, msg) => {
    if (!cond) throw new Error(`self-test failed: ${msg}`);
  };
  const types = releasableTypes({
    packages: {
      ".": {
        "changelog-sections": [
          { type: "feat", section: "Features" },
          { type: "fix", section: "Bug Fixes" },
          { type: "perf", section: "Performance" },
          { type: "docs", section: "Documentation", hidden: true },
        ],
      },
    },
  });
  const c = (subject, body = "") => ({ subject, body });
  assert(types.has("fix") && types.has("perf") && !types.has("docs") && !types.has("feat"), "releasable types");
  assert(bumpOf(c("feat: x"), types) === "minor", "feat is minor");
  assert(bumpOf(c("feat(app): x"), types) === "minor", "scope ignored");
  assert(bumpOf(c("fix: x"), types) === "patch", "fix is patch");
  assert(bumpOf(c("perf(core): x"), types) === "patch", "perf is patch");
  assert(bumpOf(c("docs: x"), types) === "none", "hidden type releases nothing");
  assert(bumpOf(c("Fix the thing"), types) === "none", "plain subject releases nothing");
  assert(bumpOf(c("fix!: x"), types) === "major", "bang is breaking");
  assert(bumpOf(c("chore: x", "BREAKING CHANGE: gone"), types) === "major", "footer is breaking");
  assert(bumpOf(c("fix:x"), types) === "none", "a subject needs the space after the colon");

  assert(apply("0.1.42", "minor") === "0.2.0", "minor resets patch");
  assert(apply("0.1.42", "major") === "0.2.0", "breaking pre-1.0 is minor");
  assert(apply("1.4.2", "major") === "2.0.0", "breaking after 1.0 is major");
  assert(apply("1.4.2", "patch") === "1.4.3", "patch");

  const many = [c("feat: a"), c("fix: b"), c("perf: c"), c("docs: d"), c("wip")];
  const p = plan("0.1.42", many, types);
  assert(p.next === "0.2.0" && p.bump === "minor", "feat wins over fix");
  assert(p.counts.feat === 1 && p.counts.other === 1, "counts by type");
  assert(plan("0.1.42", [c("docs: d")], types).next === null, "nothing releasable");

  assert(compare("0.2.0", "0.1.99") === 1 && compare("0.10.0", "0.9.0") === 1, "numeric compare");
  assert(local(p) === "0.2.0", "local = next");
  assert(local(p, "0.1.42") === "0.2.0", "next already above the installed one");
  assert(local(p, "0.2.0") === "0.2.1", "installed equals next: one patch above");
  assert(local(p, "0.2.3") === "0.2.4", "installed above next: one patch above it");
  assert(local(plan("0.1.42", [], types)) === "0.1.43", "nothing releasable: base + patch");
  console.log("next-version self-test ok");
}

function main() {
  const args = process.argv.slice(2);
  const arg = (name) => {
    const i = args.indexOf(`--${name}`);
    return i !== -1 && i + 1 < args.length ? args[i + 1] : undefined;
  };
  if (args.includes("--self-test")) return selfTest();
  const root = resolve(arg("root") ?? join(dirname(fileURLToPath(import.meta.url)), ".."));
  const above = arg("above");
  if (above) parseVersion(above);
  const p = compute(root);
  if (args.includes("--json")) return console.log(JSON.stringify({ ...p, local: local(p, above) }, null, 2));
  console.log(args.includes("--local") || above ? local(p, above) : (p.next ?? p.base));
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    main();
  } catch (e) {
    console.error(`next-version: ${e.message}`);
    process.exit(1);
  }
}
