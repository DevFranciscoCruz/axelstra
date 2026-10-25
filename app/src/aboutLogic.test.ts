import { afterEach, describe, expect, it } from "vitest";
import type { Diagnostics, HostInfoReport } from "./api";
import { aboutInfo, aboutRows, aboutText, chipText, fmtBuilt, restartHelps, skewWarning } from "./aboutLogic";
import { LOCALES, setLocale } from "./i18n";

const diag = (o: Partial<Diagnostics> = {}): Diagnostics => ({
  app_version: "0.3.1",
  app_commit: "a1b2c3d",
  app_built: "2026-10-25",
  os: "windows x86_64",
  webview: "155.0.1",
  app_proto: 7,
  daemon_connected: true,
  daemon_proto: 7,
  daemon_instance: 1,
  daemon_build: "0.3.1+axelstra-daemon",
  daemon_security: 2,
  app_security_min: 2,
  data_dir: "C:/d",
  lag_log: "C:/d/lag.log",
  lag_log_bytes: 0,
  missing_tools: [],
  ...o,
});
const host = (o: Partial<HostInfoReport> = {}) =>
  ({
    proto: 7,
    build: "0.3.1+axelstra-daemon",
    version: "0.3.1",
    commit: "a1b2c3d",
    built: "2026-10-25",
    ...o,
  }) as HostInfoReport;

afterEach(() => setLocale("pt-PT"));

describe("aboutInfo", () => {
  it("same version and commit is in sync, no warning", () => {
    const a = aboutInfo(diag(), host());
    expect(a.skew).toBe("inSync");
    expect(skewWarning(a)).toBeNull();
    expect(restartHelps(a)).toBe(false);
  });
  it("an older daemon warns and a restart helps", () => {
    const a = aboutInfo(diag({ daemon_build: "0.3.0+x" }), host({ version: "0.3.0" }));
    expect(a.skew).toBe("older");
    expect(skewWarning(a)).toContain("v0.3.0");
    expect(restartHelps(a)).toBe(true);
  });
  it("a daemon without version fields falls back to its build id; commit and date are unknown", () => {
    const a = aboutInfo(diag(), null);
    expect(a.daemonVersion).toBe("0.3.1");
    expect(a.daemonCommit).toBeNull();
    expect(a.skew).toBe("inSync");
  });
  it("a newer daemon asks for an app update, not a restart", () => {
    const a = aboutInfo(diag({ daemon_build: "0.4.0+x" }), host({ version: "0.4.0" }));
    expect(a.skew).toBe("newer");
    expect(restartHelps(a)).toBe(false);
  });
  it("same version, different commit differs", () => {
    const a = aboutInfo(diag(), host({ commit: "ffffff0" }));
    expect(a.skew).toBe("differs");
    expect(restartHelps(a)).toBe(true);
  });
  it("an unknown commit never counts as a difference", () => {
    expect(aboutInfo(diag({ app_commit: "unknown" }), host({ commit: "ffffff0" })).skew).toBe("inSync");
  });
  it("no daemon connection", () => {
    const a = aboutInfo(diag({ daemon_connected: false, daemon_proto: null, daemon_build: null }), null);
    expect(a.skew).toBe("noDaemon");
    expect(aboutRows(a).daemon).toHaveLength(1);
  });
});

describe("texts", () => {
  it("the chip is version and commit, or just the version", () => {
    expect(chipText({ appVersion: "0.1.0", appCommit: "a1b2c3d" })).toBe("v0.1.0 · a1b2c3d");
    expect(chipText({ appVersion: "0.1.0", appCommit: null })).toBe("v0.1.0");
  });
  it("the build date follows the UI language and is the UTC day", () => {
    setLocale("en");
    expect(fmtBuilt("2026-10-25")).toContain("2026");
    expect(fmtBuilt("2026-10-25")).toMatch(/Oct/);
    setLocale("pt-PT");
    expect(fmtBuilt("2026-10-25")).toBe("25/10/2026");
    expect(fmtBuilt(null)).toBe("desconhecido");
    expect(fmtBuilt("sometime")).toBe("sometime");
  });
  it("the copied text has app and daemon facts", () => {
    const t = aboutText(aboutInfo(diag(), host()));
    for (const s of ["0.3.1", "a1b2c3d", "windows x86_64", "155.0.1", "0.3.1+axelstra-daemon", "7"])
      expect(t).toContain(s);
  });
  it("every language has the warning texts", () => {
    for (const l of LOCALES) {
      setLocale(l);
      expect(skewWarning(aboutInfo(diag({ daemon_build: "0.3.0+x" }), null))).toContain("0.3.0");
    }
  });
});
