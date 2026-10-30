import { afterEach, describe, expect, it } from "vitest";
import type { DiskStatus, DiskWorktree } from "./api";
import { STALE_MS, bySize, clampInt, fmtMb, isStale, reclaimableTotal, staleList, wtName } from "./diskLogic";
import { setLocale } from "./i18n";
import { D } from "./strings.disk";
import { ago } from "./time";

afterEach(() => setLocale("pt-PT"));

const NOW = 1_800_000_000_000;
const wt = (o: Partial<DiskWorktree> = {}): DiskWorktree => ({
  project: "p",
  path: "/r/.axelstra-wt/x",
  branch: "ax/x",
  total_mb: 1000,
  target_mb: 800,
  node_modules_mb: 100,
  cache_mb: 0,
  last_used_ms: NOW - 2 * STALE_MS,
  integrated: false,
  ahead: 3,
  sessions: [],
  active: false,
  reclaimable_mb: 900,
  ...o,
});
const status = (worktrees: DiskWorktree[]): DiskStatus => ({ volumes: [], worktrees, measured_ms: NOW });

describe("fmtMb", () => {
  it("writes MB below a GB, one decimal below 10 GB, whole GB above", () => {
    expect(fmtMb(0)).toBe("0 MB");
    expect(fmtMb(512.4)).toBe("512 MB");
    expect(fmtMb(9318)).toBe("9,1 GB");
    expect(fmtMb(20480)).toBe("20 GB");
    expect(fmtMb(-5)).toBe("0 MB");
  });
  it("follows the language's decimal mark", () => {
    setLocale("en");
    expect(fmtMb(9318)).toBe("9.1 GB");
    expect(fmtMb(1_048_576)).toBe("1,024 GB");
  });
});

describe("isStale", () => {
  it("calls out unintegrated work idle for more than a day", () => {
    expect(isStale(wt(), NOW)).toBe(true);
  });
  it("leaves out integrated work, no own commits, recent use, unknown use and worktrees in use", () => {
    expect(isStale(wt({ integrated: true }), NOW)).toBe(false);
    expect(isStale(wt({ ahead: 0 }), NOW)).toBe(false);
    expect(isStale(wt({ last_used_ms: NOW - STALE_MS + 1 }), NOW)).toBe(false);
    expect(isStale(wt({ last_used_ms: null }), NOW)).toBe(false);
    expect(isStale(wt({ active: true }), NOW)).toBe(false);
    expect(isStale(wt({ sessions: [4] }), NOW)).toBe(false);
  });
  it("lists the stale ones biggest first", () => {
    const s = status([
      wt({ path: "a", total_mb: 10 }),
      wt({ path: "b", total_mb: 30, integrated: true }),
      wt({ path: "c", total_mb: 20 }),
    ]);
    expect(staleList(s, NOW).map((w) => w.path)).toEqual(["c", "a"]);
    expect(staleList(null, NOW)).toEqual([]);
  });
});

describe("sizes", () => {
  it("sorts by size and sums only what inactive worktrees can free", () => {
    const s = status([wt({ path: "a", total_mb: 1 }), wt({ path: "b", total_mb: 5, active: true, reclaimable_mb: 0 })]);
    expect(bySize(s).map((w) => w.path)).toEqual(["b", "a"]);
    expect(reclaimableTotal(s)).toBe(900);
  });
  it("names a worktree by its branch without ax/, else its folder", () => {
    expect(wtName(wt())).toBe("x");
    expect(wtName(wt({ branch: "", path: "C:\\r\\.axelstra-wt\\y" }))).toBe("y");
  });
});

describe("warning text", () => {
  it("reads like the spec in every language", () => {
    const w = wt({ total_mb: 9216 });
    const line = () => D.stale.line(w.branch, w.ahead, "main", ago(2 * 86400), fmtMb(w.total_mb));
    expect(line()).toBe(`ax/x: 3 commits fora do main, inativo ${ago(2 * 86400)}, 9,0 GB`);
    setLocale("en");
    expect(line()).toBe(`ax/x: 3 commits not in main, idle ${ago(2 * 86400)}, 9.0 GB`);
  });
});

describe("clampInt", () => {
  it("rounds into the range and refuses what is not a number", () => {
    expect(clampInt("6", 1, 720)).toBe(6);
    expect(clampInt(" 6.6 ", 1, 720)).toBe(7);
    expect(clampInt("0", 1, 720)).toBe(1);
    expect(clampInt("9999", 1, 720)).toBe(720);
    expect(clampInt("", 1, 720)).toBeNull();
    expect(clampInt("abc", 1, 720)).toBeNull();
  });
});
