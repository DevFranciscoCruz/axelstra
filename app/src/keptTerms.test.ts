import { describe, expect, it } from "vitest";
import { KEEP, pruneKept, touchKept } from "./keptTerms";

describe("kept terminals", () => {
  it("adds the active one and evicts the least recently used past KEEP", () => {
    const last = new Map([
      [1, 10],
      [2, 20],
      [3, 30],
      [4, 40],
      [5, 50],
      [6, 60],
    ]);
    expect(touchKept([1, 2, 3], 4, last, KEEP)).toEqual([1, 2, 3, 4]);
    expect(touchKept([1, 2, 3, 4, 5], 6, last, KEEP)).toEqual([2, 3, 4, 5, 6]);
    const same = [1, 2];
    expect(touchKept(same, 2, last)).toBe(same);
  });

  it("closed ids give up their slot and their lastUsed entry (F5)", () => {
    const last = new Map([
      [1, 10],
      [2, 20],
      [3, 30],
    ]);
    const kept = [1, 2, 3];
    expect(pruneKept(kept, new Set([1, 3]), last)).toEqual([1, 3]);
    expect([...last.keys()]).toEqual([1, 3]);
    const stay = [1, 3];
    expect(pruneKept(stay, new Set([1, 3, 9]), last)).toBe(stay);
  });

  it("a live terminal is no longer evicted by a dead id", () => {
    const last = new Map([
      [1, 1],
      [2, 2],
      [3, 3],
      [4, 4],
      [5, 5],
    ]);
    // 1..5 kept, 1 and 2 were closed: opening 6 must not push out a live one
    let kept = pruneKept([1, 2, 3, 4, 5], new Set([3, 4, 5, 6]), last);
    kept = touchKept(kept, 6, last);
    expect(kept).toEqual([3, 4, 5, 6]);
  });
});
