// CLI-level smoke test of the engine binary: boot a daemon in a throwaway data dir, talk to it, stop it.
//
//   cargo build -p axelstra && node tools/smoke.mjs [path/to/axelstra]
//
// Binary: argv[2], else $AXELSTRA_BIN, else target/debug/axelstra[.exe].
// Isolation: LOCALAPPDATA, HOME and XDG_DATA_HOME point at a temp dir (that is what proto::data_dir() reads),
// and on Windows USERNAME is changed too because the pipe name is \.\pipe\axelstra-<USERNAME>, so a real
// daemon of the same user is never touched. Steps: daemon boots -> `ls` answers -> `spawn` starts a
// session that prints a marker -> `ls --json` lists it -> `kill` -> daemon stopped.
import { spawn, spawnSync } from "node:child_process";
import { existsSync, mkdtempSync, rmSync, mkdirSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const exe = process.platform === "win32" ? ".exe" : "";
const bin = resolve(process.argv[2] ?? process.env.AXELSTRA_BIN ?? join(root, "target", "debug", `axelstra${exe}`));
if (!existsSync(bin)) {
  console.error(`smoke: binary not found: ${bin} (cargo build -p axelstra)`);
  process.exit(2);
}

const dir = mkdtempSync(join(tmpdir(), "axelstra-smoke-"));
const home = join(dir, "home");
mkdirSync(home, { recursive: true });
const env = {
  ...process.env,
  HOME: home,
  USERPROFILE: home,
  LOCALAPPDATA: home,
  XDG_DATA_HOME: join(home, "share"),
  USERNAME: `axsmoke${process.pid}`,
  USER: `axsmoke${process.pid}`,
};
const cli = (...args) => spawnSync(bin, args, { env, encoding: "utf8", timeout: 20000 });
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

let daemon;
let failed = null;
try {
  daemon = spawn(bin, ["daemon"], { env, stdio: ["ignore", "ignore", "inherit"] });
  daemon.on("exit", (code) => daemon && (daemon.gone = code ?? "signal"));

  let up = false;
  for (let i = 0; i < 60 && !up; i++) {
    if (daemon.gone !== undefined) throw new Error(`daemon exited early (${daemon.gone})`);
    up = cli("ls").status === 0;
    if (!up) await sleep(500);
  }
  if (!up) throw new Error("daemon did not answer `ls` within 30s");
  console.log("ok  daemon up, `ls` answers");

  const sp = cli(
    "spawn",
    "--",
    process.execPath,
    "-e",
    "console.log('axelstra-smoke-marker'); setTimeout(()=>{}, 60000)",
  );
  if (sp.status !== 0) throw new Error(`spawn failed: ${sp.stderr || sp.stdout}`);
  const id = /\d+/.exec(sp.stdout)?.[0];
  if (!id) throw new Error(`spawn printed no session id: ${sp.stdout}`);
  console.log(`ok  spawned session ${id}`);

  const ls = cli("ls", "--json");
  if (ls.status !== 0) throw new Error(`ls --json failed: ${ls.stderr}`);
  if (!JSON.parse(ls.stdout).some((s) => String(s.id) === id))
    throw new Error(`session ${id} missing from ls --json: ${ls.stdout}`);
  console.log("ok  session listed");

  const k = cli("kill", id);
  if (k.status !== 0) throw new Error(`kill failed: ${k.stderr || k.stdout}`);
  console.log("ok  session killed");
} catch (e) {
  failed = e;
} finally {
  if (daemon && daemon.gone === undefined) daemon.kill();
  await sleep(300);
  try {
    rmSync(dir, { recursive: true, force: true });
  } catch {}
}
if (failed) {
  console.error(`smoke FAILED: ${failed.message}`);
  process.exit(1);
}
console.log("smoke passed");
