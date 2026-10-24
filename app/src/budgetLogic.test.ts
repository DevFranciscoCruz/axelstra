import { describe, expect, it } from "vitest";
import type { BudgetRow } from "./api";
import {
  MAX_LIMIT,
  formatLimit,
  hasLimits,
  levelClass,
  parseTokens,
  rowsFor,
  withLimit,
  withoutWorktree,
  worstLevel,
} from "./budgetLogic";

const WT_A = String.raw`C:\wt\a`;
const WT_B = String.raw`C:\wt\b`;
const row = (level: BudgetRow["level"], worktree = WT_A, period: BudgetRow["period"] = "daily"): BudgetRow => ({
  worktree,
  period,
  used: 1,
  limit: 2,
  pct: 50,
  level,
  auto_pause: false,
});

describe("parseTokens", () => {
  it("reads plain numbers, suffixes and pt-PT separators", () => {
    expect(parseTokens("1500000")).toBe(1_500_000);
    expect(parseTokens("5M")).toBe(5_000_000);
    expect(parseTokens(" 800k ")).toBe(800_000);
    expect(parseTokens("1,5M")).toBe(1_500_000);
    expect(parseTokens("1.5m")).toBe(1_500_000);
    expect(parseTokens("2B")).toBe(2_000_000_000);
    expect(parseTokens("1.500.000")).toBe(1_500_000);
    expect(parseTokens("5 000")).toBe(5000);
  });
  it("empty means no limit, nonsense is refused", () => {
    expect(parseTokens("")).toBeNull();
    expect(parseTokens("   ")).toBeNull();
    for (const bad of ["abc", "0", "-5", "5x", "1.5", "1,5", "M", "5MM", "0k", `${MAX_LIMIT + 1}`])
      expect(parseTokens(bad), bad).toBeUndefined();
  });
});

describe("formatLimit", () => {
  it("round-trips with parseTokens", () => {
    for (const n of [1, 999, 1000, 800_000, 1_500_000, 5_000_000, 2_000_000_000, 1_234_567])
      expect(parseTokens(formatLimit(n))).toBe(n);
    expect(formatLimit(5_000_000)).toBe("5M");
    expect(formatLimit(800_000)).toBe("800k");
    expect(formatLimit(1_234_567)).toBe("1234567");
    expect(formatLimit(undefined)).toBe("");
  });
});

describe("editing budgets", () => {
  it("sets and removes a limit without touching the rest", () => {
    let b = withLimit({}, "default", { daily: 1000 });
    b = withLimit(b, "default", { weekly: 5000 });
    expect(b.default).toEqual({ daily: 1000, weekly: 5000 });
    b = withLimit(b, "default", { daily: undefined });
    expect(b.default).toEqual({ weekly: 5000 });
  });
  it("auto_pause is only ever stored when on", () => {
    let b = withLimit({}, "default", { auto_pause: true });
    expect(b.default).toEqual({ auto_pause: true });
    b = withLimit(b, "default", { auto_pause: false });
    expect(b.default).toEqual({});
    expect(withLimit({}, "default", { daily: 5 }).default?.auto_pause).toBeUndefined();
  });
  it("a worktree entry is its own and can be removed whatever the slashes", () => {
    let b = withLimit({ default: { daily: 9 } }, { worktree: WT_A }, {});
    expect(b.worktrees).toEqual({ [WT_A]: {} });
    b = withLimit(b, { worktree: WT_A }, { daily: 3 });
    expect(b.worktrees?.[WT_A]).toEqual({ daily: 3 });
    expect(b.default).toEqual({ daily: 9 });
    b = withoutWorktree(b, "c:/wt/a/");
    expect(b.worktrees).toEqual({});
  });
  it("hasLimits tells an empty setting from a used one", () => {
    expect(hasLimits(undefined)).toBe(false);
    expect(hasLimits({})).toBe(false);
    expect(hasLimits({ default: {} })).toBe(false);
    expect(hasLimits({ default: { weekly: 1 } })).toBe(true);
    expect(hasLimits({ worktrees: { x: {} } })).toBe(true);
  });
});

describe("rows", () => {
  it("picks the rows of a worktree whatever the slashes and case", () => {
    const rows = [row("ok"), row("warn", WT_B)];
    expect(rowsFor(rows, "c:/WT/a/")).toHaveLength(1);
    expect(rowsFor(rows, "c:/wt/none")).toEqual([]);
  });
  it("the worst level wins and maps to the meter classes", () => {
    expect(worstLevel([])).toBe("ok");
    expect(worstLevel([row("ok"), row("warn")])).toBe("warn");
    expect(worstLevel([row("warn"), row("over"), row("ok")])).toBe("over");
    expect(levelClass("ok")).toEqual({ warn: false, bad: false });
    expect(levelClass("warn")).toEqual({ warn: true, bad: false });
    expect(levelClass("over")).toEqual({ warn: false, bad: true });
  });
});
