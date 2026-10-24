import { describe, expect, it } from "vitest";
import { basename, norm, toggleIn } from "./paths";

describe("paths helpers", () => {
  it("basename takes the last segment with either slash and ignores trailing ones", () => {
    expect(basename(String.raw`C:\a\b\\`)).toBe("b");
    expect(basename("/x/y/z")).toBe("z");
    expect(basename("solo")).toBe("solo");
    expect(basename(null)).toBe("");
  });
  it("norm compares paths in one form", () => {
    expect(norm(String.raw`C:\A\b\\`)).toBe(norm("c:/a/b"));
  });
  it("toggleIn flips membership without touching the original", () => {
    const a = new Set([1]);
    expect([...toggleIn(a, 2)]).toEqual([1, 2]);
    expect([...toggleIn(a, 1)]).toEqual([]);
    expect([...a]).toEqual([1]);
  });
});
