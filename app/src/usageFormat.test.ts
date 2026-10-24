import { describe, expect, it } from "vitest";
import { WtUsage, fmtTokens, ranked, spentOf, totalOf, usageTip } from "./usageFormat";

const u = (worktree: string, input: number, output: number, cache_read = 0, cache_write = 0): WtUsage => ({ worktree, input, output, cache_read, cache_write, claude_sessions: 1, codex_sessions: 0 });

describe("usageFormat", () => {
  it("shortens counts", () => {
    expect(fmtTokens(950)).toBe("950");
    expect(fmtTokens(1500)).toBe("1,5k");
    expect(fmtTokens(2_400_000)).toBe("2,4M");
  });

  it("ranks by what was paid for and drops unused worktrees", () => {
    const list = [u("a", 10, 10), u("idle", 0, 0), u("b", 500, 100, 9, 20), u("c", 0, 0, 5)];
    expect(ranked(list).map((x) => x.worktree)).toEqual(["b", "a", "c"]);
    expect(spentOf(list[2])).toBe(620);
  });

  it("totals and describes", () => {
    const list = [u("a", 1000, 500, 4000, 100), u("b", 200, 100)];
    expect(totalOf(list)).toMatchObject({ input: 1200, output: 600, cache_read: 4000, cache_write: 100, claude_sessions: 2 });
    const tip = usageTip(list, (p) => p.toUpperCase());
    expect(tip.split("\n")[0]).toContain("1,9k gastos");
    expect(tip).toContain("A: 1,6k");
    expect(usageTip([u("a", 0, 0)], String)).toMatch(/Nenhum/);
  });
});
