// Builds the Tauri updater manifest (latest.json) attached to each GitHub Release. Two modes:
//
//   fragment: one platform's entry, written by the job that built it
//     node write-latest-json.mjs --fragment --key windows-x86_64 --url https://.../setup.exe --sig-file setup.exe.sig --out frag.json
//   manifest: merges the fragments of every platform
//     node write-latest-json.mjs --version 0.2.0 --notes-file notes.md --out latest.json frag1.json frag2.json
//
// The updater needs an absolute installer URL and the installer's minisign signature per platform.
import { readFileSync, writeFileSync } from "node:fs";

const args = process.argv.slice(2);
const flag = (name) => args.includes(`--${name}`);
const arg = (name, fallback) => {
  const i = args.indexOf(`--${name}`);
  if (i === -1 || i + 1 >= args.length) {
    if (fallback !== undefined) return fallback;
    throw new Error(`write-latest-json: missing --${name}`);
  }
  return args[i + 1];
};
const write = (obj) => {
  writeFileSync(arg("out"), `${JSON.stringify(obj, null, 2)}\n`);
  console.log(`wrote ${arg("out")}`);
};

if (flag("fragment")) {
  write({ key: arg("key"), url: arg("url"), signature: readFileSync(arg("sig-file"), "utf8").trim() });
} else {
  const version = arg("version");
  if (!/^\d+\.\d+\.\d+$/.test(version)) throw new Error(`bad version ${version}`);
  // positional arguments are the fragment files: everything that is not a flag or a flag's value
  const valued = new Set(["--version", "--notes-file", "--notes", "--out"]);
  const files = args.filter((a, i) => !a.startsWith("--") && !valued.has(args[i - 1]));
  if (files.length === 0) throw new Error("write-latest-json: no fragment files given");
  const platforms = {};
  for (const f of files) {
    const { key, url, signature } = JSON.parse(readFileSync(f, "utf8"));
    platforms[key] = { url, signature };
  }
  const notesFile = arg("notes-file", "");
  write({
    version,
    notes: notesFile ? readFileSync(notesFile, "utf8") : arg("notes", ""),
    pub_date: new Date().toISOString(),
    platforms,
  });
}
