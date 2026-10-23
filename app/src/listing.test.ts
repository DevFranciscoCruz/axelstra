import { describe, expect, it } from "vitest";
import { capList, sameListing } from "./listing";

describe("sameListing", () => {
  const a = [{ name: "src", dir: true }, { name: "a.ts", dir: false }];
  it("is true for an identical listing, false for any difference", () => {
    expect(sameListing(a, [{ name: "src", dir: true }, { name: "a.ts", dir: false }])).toBe(true);
    expect(sameListing(a, [{ name: "src", dir: true }])).toBe(false);
    expect(sameListing(a, [{ name: "src", dir: true }, { name: "b.ts", dir: false }])).toBe(false);
    expect(sameListing(a, [{ name: "src", dir: false }, { name: "a.ts", dir: false }])).toBe(false);
  });
  it("a folder not loaded yet is never the same", () => {
    expect(sameListing(null, [])).toBe(false);
  });
});

describe("capList", () => {
  it("keeps a short list whole", () => {
    expect(capList([1, 2, 3], 5)).toEqual({ shown: [1, 2, 3], hidden: 0 });
  });
  it("holds back the rest of a long one", () => {
    expect(capList([1, 2, 3, 4, 5], 2)).toEqual({ shown: [1, 2], hidden: 3 });
  });
});
