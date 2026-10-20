// Writes the Tauri updater manifest. The url is relative (`releases/<v>/setup.exe`): the update
// proxy rewrites it to its own download route, so the app never sees where the file lives.
// usage: node write-latest-json.mjs --version 0.2.0 --sig-file setup.exe.sig --notes "text" --out latest.json
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
      url: `releases/${version}/setup.exe`,
      signature: readFileSync(arg("sig-file"), "utf8").trim(),
    },
  },
};
writeFileSync(arg("out"), `${JSON.stringify(manifest, null, 2)}\n`);
console.log(`wrote ${arg("out")} for ${version}`);
