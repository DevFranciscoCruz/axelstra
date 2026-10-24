import { describe, expect, it } from "vitest";
import { isMultiLine, setupCommands, setupInput } from "./setupRun";

describe("setup commands", () => {
  it("one command per line, trimmed, without blanks or comments", () => {
    expect(setupCommands("npm ci\r\n\n  # skip me\n npm run build  \n")).toEqual(["npm ci", "npm run build"]);
    expect(setupCommands("")).toEqual([]);
    expect(setupCommands(null)).toEqual([]);
    expect(setupCommands(undefined)).toEqual([]);
  });

  it("types every command followed by Enter", () => {
    expect(setupInput(["a", "b"])).toBe("a\rb\r");
    expect(setupInput([])).toBe("");
  });

  it("tells a multi-line setup from a one-line one", () => {
    expect(isMultiLine("npm ci")).toBe(false);
    expect(isMultiLine("npm ci\n")).toBe(false);
    expect(isMultiLine("npm ci\nnpm run build")).toBe(true);
    expect(isMultiLine(null)).toBe(false);
  });
});
