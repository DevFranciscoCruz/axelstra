// How many parallel compile jobs fit this machine. Pure arithmetic, so it can be tested without a compile.
//
// A rustc job needs roughly `perJobBytes` of RAM at its peak (a link of the daemon or the Tauri host is the worst one),
// so jobs are limited by free memory as much as by cores. Never fewer than 1 and never more than the cores.
// `scripts/check.mjs` and `tools/measure-build.mjs` use it; `scripts/build-installer.ps1` keeps the same formula in
// PowerShell (cores / 2, at least 2) for the release build, where memory is not read.
import { availableParallelism, freemem } from "node:os";
import { pathToFileURL } from "node:url";

export const DEFAULT_PER_JOB_BYTES = 1.5 * 1024 ** 3;

// cores: logical cores. freeMemBytes: memory that can be used now. reserveBytes: left for the OS and the editor.
export function jobsFor({ cores, freeMemBytes, perJobBytes = DEFAULT_PER_JOB_BYTES, reserveBytes = 2 * 1024 ** 3 }) {
  const c = Math.max(1, Math.floor(Number(cores) || 1));
  const usable = Math.max(0, Number(freeMemBytes) - reserveBytes) || 0;
  const byMem = Math.floor(usable / perJobBytes);
  // cores / 2: SMT siblings do not help a compile much, and a build should not make the computer sluggish
  const byCpu = Math.max(1, Math.floor(c / 2));
  return Math.max(1, Math.min(byCpu, byMem));
}

// Jobs for this machine right now. An explicit CARGO_BUILD_JOBS always wins.
export function defaultJobs(env = process.env) {
  const given = Number.parseInt(env.CARGO_BUILD_JOBS ?? "", 10);
  if (given > 0) return given;
  return jobsFor({ cores: availableParallelism(), freeMemBytes: freemem() });
}

export function selfTest() {
  const GB = 1024 ** 3;
  const eq = (got, want, msg) => {
    if (got !== want) throw new Error(`self-test failed: ${msg}: got ${got}, want ${want}`);
  };
  eq(jobsFor({ cores: 16, freeMemBytes: 32 * GB }), 8, "plenty of memory: half the cores");
  eq(jobsFor({ cores: 16, freeMemBytes: 8 * GB }), 4, "memory limits: (8-2)/1.5");
  eq(jobsFor({ cores: 16, freeMemBytes: 0 }), 1, "no memory: still 1");
  eq(jobsFor({ cores: 16, freeMemBytes: 1 * GB }), 1, "below the reserve: still 1");
  eq(jobsFor({ cores: 1, freeMemBytes: 64 * GB }), 1, "one core");
  eq(jobsFor({ cores: 2, freeMemBytes: 64 * GB }), 1, "two cores: one job");
  eq(jobsFor({ cores: 4, freeMemBytes: 64 * GB, perJobBytes: 8 * GB }), 2, "custom per-job size, cpu bound");
  eq(jobsFor({ cores: 64, freeMemBytes: 10 * GB, perJobBytes: 2 * GB, reserveBytes: 2 * GB }), 4, "custom reserve");
  eq(jobsFor({ cores: Number.NaN, freeMemBytes: Number.NaN }), 1, "garbage in: 1");
  eq(defaultJobs({ CARGO_BUILD_JOBS: "3" }), 3, "explicit env wins");
  const auto = defaultJobs({});
  if (!(auto >= 1)) throw new Error("self-test failed: defaultJobs >= 1");
  console.log("jobs self-test ok");
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  if (process.argv.includes("--self-test")) selfTest();
  else console.log(defaultJobs());
}
