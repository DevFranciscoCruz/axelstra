// Keeps the three version fields in step and ties them to the release tag.
//
//   node scripts/stamp-version.mjs --check              the three fields agree with each other
//   node scripts/stamp-version.mjs --tag v1.2.3         stamp the tag version into all three, then assert tag == version
//   node scripts/stamp-version.mjs --self-test          run the script's own tests (CI)
//   --root <dir>                                        repo root (default: the parent of scripts/)
//
// Fields: Cargo.toml [workspace.package] version, app/package.json, app/src-tauri/tauri.conf.json.
// Cargo.lock: the workspace's own packages (the ones without a `source`) are stamped too, so the lockfile a tagged
// build starts from already matches and `cargo build --locked` does not fail on it.
// Only the version value is rewritten; the rest of each file stays byte for byte.
import { readFileSync, writeFileSync, mkdtempSync, mkdirSync, rmSync, existsSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const JSON_RE = /^(\s*"version"\s*:\s*")([^"]*)(")/m;
const CARGO_RE = /(\[workspace\.package\][^[]*?\nversion\s*=\s*")([^"]*)(")/;

export const FIELDS = [
  { file: "Cargo.toml", re: CARGO_RE },
  { file: "app/package.json", re: JSON_RE },
  { file: "app/src-tauri/tauri.conf.json", re: JSON_RE },
];

// A [[package]] block of Cargo.lock; a workspace-local package has no `source` line.
const LOCK_BLOCK_RE = /(\[\[package\]\]\r?\nname = "[^"]*"\r?\nversion = ")([^"]*)("\r?\n)((?:(?!\[\[package\]\])[^\n]*\n?)*)/g;

export function lockVersions(root) {
  const path = join(root, "Cargo.lock");
  if (!existsSync(path)) return [];
  const out = [];
  for (const m of readFileSync(path, "utf8").matchAll(LOCK_BLOCK_RE)) {
    if (!/^source = /m.test(m[4])) out.push({ name: /name = "([^"]*)"/.exec(m[1])[1], version: m[2] });
  }
  return out;
}

function stampLock(root, from, to) {
  const path = join(root, "Cargo.lock");
  if (!existsSync(path)) return;
  const text = readFileSync(path, "utf8");
  const next = text.replace(LOCK_BLOCK_RE, (all, head, ver, mid, rest) =>
    !/^source = /m.test(rest) && ver === from ? `${head}${to}${mid}${rest}` : all,
  );
  if (next !== text) writeFileSync(path, next);
}

export function read(root) {
  return FIELDS.map(({ file, re }) => {
    const m = readFileSync(join(root, file), "utf8").match(re);
    if (!m) throw new Error(`${file}: no version field found`);
    return { file, version: m[2] };
  });
}

export function check(root) {
  const fields = read(root);
  const distinct = new Set(fields.map((f) => f.version));
  if (distinct.size !== 1) {
    throw new Error(`version fields disagree: ${fields.map((f) => `${f.file}=${f.version}`).join(", ")}`);
  }
  const version = fields[0].version;
  const stale = lockVersions(root).filter((p) => p.version !== version);
  if (stale.length) {
    throw new Error(`Cargo.lock disagrees with Cargo.toml (${version}): ${stale.map((p) => `${p.name}=${p.version}`).join(", ")}`);
  }
  return version;
}

export function versionOfTag(tag) {
  const m = /^v(\d+\.\d+\.\d+)$/.exec(tag);
  if (!m) throw new Error(`bad tag ${tag}: expected vX.Y.Z`);
  return m[1];
}

export function stamp(root, tag) {
  const version = versionOfTag(tag);
  const before = check(root);
  for (const { file, re } of FIELDS) {
    const path = join(root, file);
    const text = readFileSync(path, "utf8");
    if (!re.test(text)) throw new Error(`${file}: no version field found`);
    writeFileSync(path, text.replace(re, `$1${version}$3`));
  }
  stampLock(root, before, version);
  const got = check(root);
  if (got !== version) throw new Error(`tag ${tag} != stamped version ${got}`);
  return version;
}

function selfTest() {
  const assert = (cond, msg) => {
    if (!cond) throw new Error(`self-test failed: ${msg}`);
  };
  const throws = (fn, re, msg) => {
    try {
      fn();
    } catch (e) {
      assert(re.test(e.message), `${msg}: wrong error "${e.message}"`);
      return;
    }
    assert(false, `${msg}: did not throw`);
  };
  const dir = mkdtempSync(join(tmpdir(), "stamp-version-"));
  try {
    const put = (file, text) => {
      mkdirSync(dirname(join(dir, file)), { recursive: true });
      writeFileSync(join(dir, file), text);
    };
    const cargo = (v) =>
      `[workspace]\nmembers = []\n\n[workspace.package]\nedition = "2021"\nversion = "${v}"\n\n[dependencies]\nfoo = { version = "9.9.9" }\n`;
    const json = (v) =>
      `{\n  "name": "x",\n  "version": "${v}",\n  "dependencies": { "a": { "version": "9.9.9" } }\n}\n`;
    const seed = (c, p, t) => {
      put("Cargo.toml", cargo(c));
      put("app/package.json", json(p));
      put("app/src-tauri/tauri.conf.json", json(t));
    };

    const lock = (a, b) =>
      `version = 4\n\n[[package]]\nname = "app"\nversion = "${a}"\ndependencies = [\n "serde",\n]\n\n[[package]]\nname = "serde"\nversion = "1.0.0"\nsource = "registry+https://github.com/rust-lang/crates.io-index"\n\n[[package]]\nname = "core"\nversion = "${b}"\n`;
    seed("0.1.0", "0.1.0", "0.1.0");
    assert(check(dir) === "0.1.0", "agreeing fields pass");

    seed("0.1.0", "0.1.1", "0.1.0");
    throws(() => check(dir), /disagree.*package\.json=0\.1\.1/, "drift detected");

    seed("0.1.0", "0.1.0", "0.1.0");
    assert(stamp(dir, "v1.2.3") === "1.2.3", "stamp returns the version");
    assert(check(dir) === "1.2.3", "all three stamped");
    assert(
      readFileSync(join(dir, "Cargo.toml"), "utf8") === cargo("1.2.3"),
      "cargo: only the workspace version changed",
    );
    assert(
      readFileSync(join(dir, "app/package.json"), "utf8") === json("1.2.3"),
      "json: only the top-level version changed",
    );

    seed("0.1.0", "0.1.0", "0.1.0");
    for (const bad of ["1.2.3", "v1.2", "v1.2.3-rc1", "vx", ""])
      throws(() => stamp(dir, bad), /bad tag/, `bad tag "${bad}"`);
    assert(check(dir) === "0.1.0", "a bad tag leaves the files untouched");

    // Cargo.lock: only the packages without a source follow the version
    seed("0.1.0", "0.1.0", "0.1.0");
    put("Cargo.lock", lock("0.1.0", "0.1.0"));
    assert(check(dir) === "0.1.0", "lock in step passes");
    assert(lockVersions(dir).length === 2, "the registry package is not one of the workspace's");
    put("Cargo.lock", lock("0.1.0", "0.1.0").replace(/\n/g, "\r\n"));
    assert(lockVersions(dir).length === 2, "a CRLF lockfile is read too");
    put("Cargo.lock", lock("0.1.0", "0.0.9"));
    throws(() => check(dir), /Cargo\.lock disagrees.*core=0\.0\.9/, "lock drift detected");
    put("Cargo.lock", lock("0.1.0", "0.1.0"));
    stamp(dir, "v2.0.0");
    assert(readFileSync(join(dir, "Cargo.lock"), "utf8") === lock("2.0.0", "2.0.0").replace('"1.0.0"', '"1.0.0"'), "lock: workspace packages stamped, registry ones untouched");
    rmSync(join(dir, "Cargo.lock"));

    put("Cargo.toml", "[workspace]\n");
    throws(() => check(dir), /Cargo\.toml: no version field/, "missing field");
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
  console.log("stamp-version self-test ok");
}

function main() {
  const args = process.argv.slice(2);
  const arg = (name) => {
    const i = args.indexOf(`--${name}`);
    return i !== -1 && i + 1 < args.length ? args[i + 1] : undefined;
  };
  const root = resolve(arg("root") ?? join(dirname(fileURLToPath(import.meta.url)), ".."));
  if (args.includes("--self-test")) return selfTest();
  if (args.includes("--check")) return console.log(`versions agree: ${check(root)}`);
  const tag = arg("tag");
  if (tag !== undefined) return console.log(`stamped ${stamp(root, tag)}`);
  throw new Error("usage: stamp-version.mjs --check | --tag vX.Y.Z | --self-test [--root dir]");
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    main();
  } catch (e) {
    console.error(`stamp-version: ${e.message}`);
    process.exit(1);
  }
}
