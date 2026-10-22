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
  const info = { app_version: "0.3.1", app_proto: 7, daemon_connected: true, daemon_proto: 7, daemon_instance: 42, data_dir: "C:/d", lag_log: "C:/d/lag.log", lag_log_bytes: 0, missing_tools: [] };
  it("lists what was reported", () => {
    expect(diagnosticRows(null)).toEqual([]);
    expect(diagnosticRows(info)).toEqual([
      { label: "Aplicação", value: "0.3.1 · protocolo 7" },
      { label: "Daemon", value: "protocolo 7 · instância 42" },
      { label: "Pasta de dados", value: "C:/d" },
      { label: "Registo de lag", value: "C:/d/lag.log" },
    ]);
  });
  it("says when the daemon is not connected and sizes a lag log", () => {
    const rows = diagnosticRows({ ...info, daemon_connected: false, daemon_proto: null, daemon_instance: null, lag_log_bytes: 2048 });
    expect(rows[1].value).toBe("sem ligação");
    expect(rows[3].value).toBe("C:/d/lag.log (2 KB)");
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
