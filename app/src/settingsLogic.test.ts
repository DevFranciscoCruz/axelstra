import { describe, expect, it, vi } from "vitest";
import { nextRadio } from "./a11y";
import { classifySettingsError, diagnosticRows, errorText, waitUntil } from "./settingsLogic";
import { DEFAULT_PREFS, clampFont, parsePrefs, zoomFont } from "./termprefs";

describe("classifySettingsError", () => {
  it("recognises an old daemon only by its unknown-method answer", () => {
    expect(classifySettingsError("unknown method: settings.get")).toBe("old-daemon");
    expect(classifySettingsError(new Error("Unknown Method settings.get"))).toBe("old-daemon");
    expect(classifySettingsError("a ligação ao daemon caiu")).toBe("other");
    expect(classifySettingsError(undefined)).toBe("other");
  });
  it("errorText drops the Error prefix", () => {
    expect(errorText(new Error("boom"))).toBe("boom");
    expect(errorText("Error: boom")).toBe("boom");
  });
});

describe("waitUntil", () => {
  it("retries until the probe works", async () => {
    const probe = vi.fn().mockRejectedValueOnce(new Error("x")).mockRejectedValueOnce(new Error("x")).mockResolvedValue(1);
    expect(await waitUntil(probe, 5, 1, () => Promise.resolve())).toBe(true);
    expect(probe).toHaveBeenCalledTimes(3);
  });
  it("gives up after the tries", async () => {
    const probe = vi.fn().mockRejectedValue(new Error("x"));
    expect(await waitUntil(probe, 3, 1, () => Promise.resolve())).toBe(false);
    expect(probe).toHaveBeenCalledTimes(3);
  });
});

describe("diagnosticRows", () => {
  it("lists what was reported and nothing else", () => {
    expect(diagnosticRows(null)).toEqual([]);
    expect(diagnosticRows({ daemon: { version: "0.3.1", proto: 7 }, paths: { data: "C:/d" }, lag_log: "C:/d/lag.log" })).toEqual([
      { label: "daemon", value: "0.3.1 · proto 7" },
      { label: "data", value: "C:/d" },
      { label: "lag.log", value: "C:/d/lag.log" },
    ]);
    expect(diagnosticRows({})).toEqual([]);
  });
});

describe("nextRadio", () => {
  it("moves with the arrows and wraps", () => {
    expect(nextRadio(0, 3, "ArrowRight")).toBe(1);
    expect(nextRadio(2, 3, "ArrowDown")).toBe(0);
    expect(nextRadio(0, 3, "ArrowLeft")).toBe(2);
    expect(nextRadio(1, 3, "Home")).toBe(0);
    expect(nextRadio(1, 3, "End")).toBe(2);
  });
  it("ignores other keys and empty groups", () => {
    expect(nextRadio(0, 3, "a")).toBeNull();
    expect(nextRadio(0, 0, "ArrowRight")).toBeNull();
  });
});

describe("terminal prefs", () => {
  it("falls back to defaults for missing or broken data", () => {
    expect(parsePrefs(null)).toEqual(DEFAULT_PREFS);
    expect(parsePrefs("{nope")).toEqual(DEFAULT_PREFS);
    expect(parsePrefs('{"theme":"light","fontSize":99,"fontFamily":" Fira Code "}')).toEqual({ theme: "light", fontSize: 32, fontFamily: "Fira Code" });
  });
  it("clamps and zooms the font", () => {
    expect(clampFont(2)).toBe(8);
    expect(clampFont(NaN)).toBe(DEFAULT_PREFS.fontSize);
    expect(zoomFont(13, 1)).toBe(14);
    expect(zoomFont(8, -1)).toBe(8);
    expect(zoomFont(20, 0)).toBe(DEFAULT_PREFS.fontSize);
  });
});
