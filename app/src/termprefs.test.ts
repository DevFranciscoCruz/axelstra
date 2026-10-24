import { afterEach, describe, expect, it, vi } from "vitest";
import { cursorBlinks, parsePrefs } from "./termprefs";

describe("terminal accessibility prefs (U11)", () => {
  afterEach(() => vi.unstubAllGlobals());
  it("screenReader is off unless explicitly true", () => {
    expect(parsePrefs(null).screenReader).toBe(false);
    expect(parsePrefs('{"screenReader":"yes"}').screenReader).toBe(false);
    expect(parsePrefs('{"screenReader":true}').screenReader).toBe(true);
  });
  it("the cursor does not blink under prefers-reduced-motion", () => {
    vi.stubGlobal("matchMedia", (q: string) => ({ matches: q.includes("reduced-motion") }));
    expect(cursorBlinks()).toBe(false);
    vi.stubGlobal("matchMedia", () => ({ matches: false }));
    expect(cursorBlinks()).toBe(true);
  });
});
