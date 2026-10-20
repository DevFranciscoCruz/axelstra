// Writes the Tauri updater manifest (latest.json) attached to each GitHub Release.
// The updater needs an absolute installer URL and the installer's minisign signature.
// usage: node write-latest-json.mjs --version 0.2.0 --sig-file setup.exe.sig --url https://github.com/.../setup.exe --notes "text" --out latest.json
import { readFileSync, writeFileSync } from "node:fs";

const arg = (name, fallback) => {
  const i = process.argv.indexOf(`--${name}`);
  if (i === -1 || i + 1 >= process.argv.length) {
    if (fallback !== undefined) return fallback;
    throw new Error(`write-latest-json: missing --${name}`);
  }
  return process.argv[i + 1];
};

const version = arg("version");
if (!/^\d+\.\d+\.\d+$/.test(version)) throw new Error(`bad version ${version}`);
const manifest = {
  version,
  notes: arg("notes", ""),
  pub_date: new Date().toISOString(),
  platforms: {
    "windows-x86_64": {
      url: arg("url"),
      signature: readFileSync(arg("sig-file"), "utf8").trim(),
    },
  },
};
writeFileSync(arg("out"), `${JSON.stringify(manifest, null, 2)}
`);
console.log(`wrote ${arg("out")} for ${version}`);
