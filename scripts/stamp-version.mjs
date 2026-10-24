// Keeps the three version fields in step and ties them to the release tag.
//
//   node scripts/stamp-version.mjs --check              the three fields agree with each other
//   node scripts/stamp-version.mjs --tag v1.2.3         stamp the tag version into all three, then assert tag == version
//   node scripts/stamp-version.mjs --self-test          run the script's own tests (CI)
//   --root <dir>                                        repo root (default: the parent of scripts/)
//
// Fields: Cargo.toml [workspace.package] version, app/package.json, app/src-tauri/tauri.conf.json.
// Only the version value is rewritten; the rest of each file stays byte for byte.
import { readFileSync, writeFileSync, mkdtempSync, mkdirSync, rmSync } from "node:fs";
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
  return fields[0].version;
}

export function versionOfTag(tag) {
  const m = /^v(\d+\.\d+\.\d+)$/.exec(tag);
  if (!m) throw new Error(`bad tag ${tag}: expected vX.Y.Z`);
  return m[1];
}

export function stamp(root, tag) {
  const version = versionOfTag(tag);
  for (const { file, re } of FIELDS) {
    const path = join(root, file);
    const text = readFileSync(path, "utf8");
    if (!re.test(text)) throw new Error(`${file}: no version field found`);
    writeFileSync(path, text.replace(re, `$1${version}$3`));
  }
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
