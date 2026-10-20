// Writes the Tauri updater manifest. The updater needs an absolute installer URL.
// usage: node write-latest-json.mjs --version 0.2.0 --sig-file setup.exe.sig --base-url https://updates.example/ --notes "text" --out latest.json
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
const base = arg("base-url").replace(/\/+$/, "");
const manifest = {
  version,
  notes: arg("notes", ""),
  pub_date: new Date().toISOString(),
  platforms: {
    "windows-x86_64": {
      url: `${base}/releases/${version}/setup.exe`,
      signature: readFileSync(arg("sig-file"), "utf8").trim(),
    },
  },
};
writeFileSync(arg("out"), `${JSON.stringify(manifest, null, 2)}\n`);
console.log(`wrote ${arg("out")} for ${version}`);
