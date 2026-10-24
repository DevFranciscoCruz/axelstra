import { describe, expect, it } from "vitest";
import { overlapCount, overlapEntries, overlapTitle, rebaseBlocker, rebaseOutcome } from "./rebase";

const map = {
  "C:\\w\\one": [
    { with: "C:\\w\\two", files: ["a.rs", "b.rs"] },
    { with: "C:\\w\\three", files: ["b.rs", "c.rs"] },
  ],
};

describe("overlap files", () => {
  it("looks a worktree up whatever the slash style", () => {
    expect(overlapEntries(map, "c:/w/one/")).toHaveLength(2);
    expect(overlapEntries(map, "c:/w/none")).toEqual([]);
    expect(overlapEntries(null, "c:/w/one")).toEqual([]);
  });

  it("counts a file shared with two worktrees once", () => {
    expect(overlapCount(overlapEntries(map, "c:/w/one"))).toBe(3);
    expect(overlapCount([])).toBe(0);
  });

  it("lists the other worktrees with their files and sums up the rest", () => {
    const t = overlapTitle(overlapEntries(map, "c:/w/one"))!;
    expect(t).toContain("two: a.rs, b.rs");
    expect(t).toContain("three: b.rs, c.rs");
    const many = overlapTitle([{ with: "/w/x", files: ["1", "2", "3", "4", "5", "6"] }])!;
    expect(many).toContain("x: 1, 2, 3, 4 e mais 2");
    expect(overlapTitle([])).toBeNull();
  });
});

describe("rebase messages", () => {
  it("tells moved from already up to date", () => {
    expect(rebaseOutcome("t", "main", { ok: true, from: "a", to: "b" })).toMatchObject({ kind: "info" });
    expect(rebaseOutcome("t", "main", { ok: true, from: "a", to: "a" }).text).toContain("já estava");
  });

  it("lists the conflicted files and says whether the branch was restored", () => {
    const o = rebaseOutcome("t", "main", { ok: false, conflicted: ["a.txt", "b.txt"], restored: true });
    expect(o.kind).toBe("warn");
    expect(o.text).toContain("a.txt, b.txt");
    expect(o.text).toContain("ficou como estava");
    expect(rebaseOutcome("t", "main", { ok: false, conflicted: ["a"], restored: false }).text).toContain("não foi possível");
  });

  it("blocks a busy or dirty worktree before asking", () => {
    expect(rebaseBlocker("t", { dirty: 0 }, 0)).toBeNull();
    expect(rebaseBlocker("t", undefined, 0)).toBeNull();
    expect(rebaseBlocker("t", { dirty: 2 }, 0)).toContain("alterações");
    expect(rebaseBlocker("t", { dirty: 0 }, 1)).toContain("a trabalhar");
  });
});
