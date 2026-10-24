// Guards the webview's security config (CSP, capabilities). Run: node --test scripts/check-tauri-config.mjs
import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..", "app", "src-tauri");
const json = (f) => JSON.parse(readFileSync(join(root, f), "utf8"));
const conf = json("tauri.conf.json");
const caps = json("capabilities/default.json");
const directives = (csp) =>
  Object.fromEntries(
    csp
      .split(";")
      .map((d) => d.trim())
      .filter(Boolean)
      .map((d) => {
        const [k, ...v] = d.split(/\s+/);
        return [k, v];
      }),
  );

test("production CSP is locked down", () => {
  const d = directives(conf.app.security.csp);
  assert.deepEqual(d["script-src"], ["'self'"]);
  assert.deepEqual(d["object-src"], ["'none'"]);
  assert.deepEqual(d["base-uri"], ["'none'"]);
  assert.deepEqual(d["frame-src"], ["'none'"]);
  assert.deepEqual(d["form-action"], ["'none'"]);
  assert.deepEqual(d["frame-ancestors"], ["'none'"]);
  assert.ok(!/localhost:d+/.test(conf.app.security.csp), "no dev origins in the production CSP");
  assert.ok(!d["script-src"].includes("'unsafe-inline'") && !d["script-src"].includes("'unsafe-eval'"));
});

test("img-src only allows the avatar hosts", () => {
  const hosts = directives(conf.app.security.csp)["img-src"].filter((s) => s.startsWith("http"));
  assert.deepEqual(hosts.sort(), ["https://avatars.githubusercontent.com", "https://github.com"]);
});

test("capability grants no broad defaults", () => {
  for (const p of caps.permissions) {
    const id = typeof p === "string" ? p : p.identifier;
    assert.ok(!/^(core|dialog|opener):default$/.test(id), `${id} is too broad`);
  }
  const open = caps.permissions.find((p) => p.identifier === "opener:allow-open-url");
  assert.ok(open, "open-url must be scoped explicitly");
  for (const a of open.allow) assert.match(a.url, /^https?:\/\//, "only http(s) may be opened");
});
