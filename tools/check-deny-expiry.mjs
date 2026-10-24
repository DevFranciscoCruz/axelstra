// Fails when an ignore in deny.toml has an "expires YYYY-MM-DD" date in the past, or has none.
//   node tools/check-deny-expiry.mjs [--today YYYY-MM-DD]   (--today is for tests)
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const i = process.argv.indexOf("--today");
const today = i > 0 ? process.argv[i + 1] : new Date().toISOString().slice(0, 10);
const text = readFileSync(join(root, "deny.toml"), "utf8");

let bad = 0;
const entries = [...text.matchAll(/\{\s*id\s*=\s*"([^"]+)"[^}]*\}/g)];
for (const [whole, id] of entries) {
  const m = /expires\s+(\d{4}-\d{2}-\d{2})/.exec(whole);
  if (!m) {
    console.error(`${id}: no "expires YYYY-MM-DD" in its reason`);
    bad++;
  } else if (m[1] < today) {
    console.error(`${id}: ignore expired on ${m[1]}; re-check the advisory, then renew or drop it`);
    bad++;
  }
}
if (!entries.length) console.log("no advisory ignores in deny.toml");
else if (!bad) console.log(`${entries.length} advisory ignore(s), none expired (today ${today})`);
process.exit(bad ? 1 : 0);
