import { describe, expect, it } from "vitest";
import type { MemStatus } from "./api";
import {
  applyPaused,
  applyPressure,
  applyResumed,
  applyStatus,
  breakdownRows,
  commitPair,
  cpuSplit,
  kernelHigh,
  kernelPoolProblem,
  chipVisible,
  initialMem,
  levelClass,
  levelTransition,
  mbParts,
  parseAmount,
  pausedDelta,
  thresholdProblem,
} from "./memoryLogic";

const status = (over: Partial<MemStatus> = {}): MemStatus => ({
  level: "normal",
  free_mb: 8000,
  total_mb: 16000,
  commit_pct: null,
  thresholds: {},
  config: {
    enabled: true,
    admission: true,
    auto_pause: false,
    warn_only: false,
    high_free_pct: 15,
    crit_free_pct: 8,
    high_free_mb: 2048,
    crit_free_mb: 1024,
  },
  sessions: [],
  paused: [],
  ...over,
});

describe("memory slice", () => {
  it("mem.status fills the whole state, biggest session first", () => {
    const s = applyStatus(
      initialMem(),
      status({
        sessions: [
          { id: 1, rss_mb: 100 },
          { id: 2, rss_mb: 900 },
        ],
      }),
    );
    expect(s.loaded).toBe(true);
    expect(s.top.map((t) => t.session)).toEqual([2, 1]);
    expect(s.config?.admission).toBe(true);
  });

  it("memory.pressure moves level, free memory and top list", () => {
    const s = applyPressure(applyStatus(initialMem(), status()), {
      level: "critical",
      free_mb: 500,
      total_mb: 16000,
      top: [{ session: 4, rss_mb: 3000 }],
    });
    expect(s).toMatchObject({ level: "critical", free_mb: 500, top: [{ session: 4, rss_mb: 3000 }] });
  });

  it("memory.pressure without a known level changes nothing", () => {
    const s = applyStatus(initialMem(), status());
    expect(applyPressure(s, { level: "bogus" })).toBe(s);
  });

  it("session.paused adds an item, session.resumed takes it away", () => {
    const s0 = applyStatus(initialMem(), status());
    const s1 = applyPaused(s0, { id: 7, build: null, reason: "memory" }, 1000);
    expect(s1.paused).toEqual([{ kind: "session", session: 7, reason: "memory", since: 1000, name: null }]);
    expect(applyPaused(s1, { id: 7, reason: "memory" }, 2000).paused).toHaveLength(1);
    expect(applyPaused(s1, { id: 8, build: "cargo test", reason: "memory" }, 3000).paused[1].kind).toBe("build");
    expect(applyResumed(s1, { id: 7 }).paused).toEqual([]);
    expect(applyResumed(s1, { id: 99 })).toBe(s1);
  });

  it("announces a level change only after the first reading", () => {
    const first = applyStatus(initialMem(), status({ level: "high" }));
    expect(levelTransition(initialMem(), first)).toBeNull();
    const next = applyPressure(first, { level: "critical", free_mb: 100 });
    expect(levelTransition(first, next)).toBe("critical");
    expect(levelTransition(next, applyPressure(next, { level: "critical", free_mb: 90 }))).toBeNull();
  });

  it("pausedDelta lists what was frozen and what came back", () => {
    const a = applyPaused(applyStatus(initialMem(), status()), { id: 1, reason: "memory" }, 1);
    const b = applyResumed(applyPaused(a, { id: 2, reason: "memory" }, 2), { id: 1 });
    const d = pausedDelta(a, b);
    expect(d.paused.map((p) => p.session)).toEqual([2]);
    expect(d.resumed.map((p) => p.session)).toEqual([1]);
  });
});

describe("presentation", () => {
  it("maps level to css class", () => {
    expect(levelClass("normal")).toBe("mem-normal");
    expect(levelClass("high")).toBe("mem-high");
    expect(levelClass("critical")).toBe("mem-critical");
  });

  it("shows the chip under pressure or with something paused", () => {
    const s = applyStatus(initialMem(), status());
    expect(chipVisible(initialMem())).toBe(false);
    expect(chipVisible(s)).toBe(false);
    expect(chipVisible({ ...s, level: "high" })).toBe(true);
    expect(chipVisible(applyPaused(s, { id: 1, reason: "manual" }, 1))).toBe(true);
  });

  it("splits megabytes into MB or GB", () => {
    expect(mbParts(512.4)).toEqual({ value: 512, digits: 0, unit: "MB" });
    expect(mbParts(1536)).toEqual({ value: 1.5, digits: 1, unit: "GB" });
  });
});

describe("threshold validation", () => {
  const ok = { high_free_pct: 15, crit_free_pct: 8, high_free_mb: 2048, crit_free_mb: 1024 };
  it("accepts critical below high", () => expect(thresholdProblem(ok)).toBeNull());
  it("refuses critical at or above high", () => {
    expect(thresholdProblem({ ...ok, crit_free_pct: 15 })).toBe("pct_order");
    expect(thresholdProblem({ ...ok, crit_free_mb: 4096 })).toBe("mb_order");
  });
  it("refuses out-of-range values", () => {
    expect(thresholdProblem({ ...ok, high_free_pct: 100 })).toBe("pct_range");
    expect(thresholdProblem({ ...ok, crit_free_pct: 0 })).toBe("pct_range");
    expect(thresholdProblem({ ...ok, crit_free_mb: 0 })).toBe("mb_range");
    expect(thresholdProblem({ ...ok, high_free_mb: NaN })).toBe("mb_range");
  });
  it("parses typed numbers", () => {
    expect(parseAmount(" 12 ")).toBe(12);
    expect(parseAmount("7,5")).toBe(7.5);
    expect(parseAmount("abc")).toBeNull();
    expect(parseAmount("")).toBeNull();
    expect(parseAmount("-3")).toBeNull();
  });
});

describe("memory breakdown", () => {
  const b = {
    kernel: { paged_mb: 300, nonpaged_mb: 4000, total_mb: 4300, baseline_mb: 800, high: true },
    cache_mb: 1000,
    compressed_mb: null,
    sessions: [
      { tier: "agent", count: 2, rss_mb: 1500 },
      { tier: "worker", count: 1, rss_mb: 500 },
    ],
    builds: { count: 1, rss_mb: 2000 },
    external: [{ name: "MsMpEng.exe", count: 1, rss_mb: 800 }],
    others_mb: 0,
    handles: { total: 300000, top: [{ name: "x.exe", pid: 7, handles: 90000 }] },
  };

  it("applyStatus keeps the breakdown, and none stays null", () => {
    expect(applyStatus(initialMem(), status({ breakdown: b })).breakdown).toEqual(b);
    expect(applyStatus(initialMem(), status()).breakdown).toBeNull();
  });

  it("rows are biggest first, empty categories are dropped, pct is of the total", () => {
    const rows = breakdownRows(b, 16000);
    expect(rows.map((r) => r.key)).toEqual([
      "kernel_nonpaged",
      "sessions",
      "builds",
      "cache",
      "external",
      "kernel_paged",
    ]);
    expect(rows[0].pct).toBeCloseTo(25, 5);
    expect(breakdownRows(null, 16000)).toEqual([]);
    expect(breakdownRows(b, 0)[0].pct).toBe(0);
  });

  it("the suggestion shows only when the kernel is high", () => {
    expect(kernelHigh(b)).toBe(true);
    expect(kernelHigh({ ...b, kernel: { ...b.kernel, high: false } })).toBe(false);
    expect(kernelHigh({ ...b, kernel: null })).toBe(false);
    expect(kernelHigh(null)).toBe(false);
  });

  it("kernel_pool_mb range is 256..=1048576", () => {
    expect(kernelPoolProblem(255)).toBe(true);
    expect(kernelPoolProblem(256)).toBe(false);
    expect(kernelPoolProblem(1048576)).toBe(false);
    expect(kernelPoolProblem(1048577)).toBe(true);
    expect(kernelPoolProblem(300.5)).toBe(true);
  });
});

describe("memory breakdown system numbers", () => {
  it("cpu and commit need both numbers; absence gives null", () => {
    const b = { system: { cpu_user_pct: 12, cpu_kernel_pct: 30, commit_mb: 20000, commit_limit_mb: 40000 } };
    expect(cpuSplit(b)).toEqual({ user: 12, kernel: 30 });
    expect(commitPair(b)).toEqual({ used: 20000, limit: 40000 });
    expect(cpuSplit({ system: { cpu_user_pct: 1 } })).toBeNull();
    expect(commitPair({ system: { commit_mb: 1, commit_limit_mb: 0 } })).toBeNull();
    expect(cpuSplit(null)).toBeNull();
    expect(commitPair({})).toBeNull();
  });
});
