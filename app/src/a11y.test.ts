import { describe, expect, it } from "vitest";
import { moveIndex } from "./a11y";

describe("moveIndex", () => {
  it("steps and stops at the ends", () => {
    expect(moveIndex(0, 3, "ArrowDown")).toBe(1);
    expect(moveIndex(2, 3, "ArrowDown")).toBe(2);
    expect(moveIndex(0, 3, "ArrowUp")).toBe(0);
    expect(moveIndex(2, 3, "ArrowUp")).toBe(1);
  });
  it("jumps with Home and End", () => {
    expect(moveIndex(1, 5, "Home")).toBe(0);
    expect(moveIndex(1, 5, "End")).toBe(4);
  });
  it("ignores other keys and empty lists", () => {
    expect(moveIndex(0, 3, "a")).toBeNull();
    expect(moveIndex(0, 0, "ArrowDown")).toBeNull();
  });
});
