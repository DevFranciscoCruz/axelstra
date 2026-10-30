import { describe, expect, it } from "vitest";
import type { Diagnostics } from "./api";
import { buildLabel, daemonOlderThanApp, parseBuild, shortCommit, staleInfo } from "./skewLogic";

const base = {
  app_version: "0.1.40",
  app_proto: 16,
  daemon_connected: true,
  daemon_proto: 16,
  daemon_build: "0.1.40+axelstra-daemon",
  daemon_security: 2,
  app_security_min: 2,
};

describe("version skew", () => {
  it("parses the version out of a build id", () => {
    expect(parseBuild("0.1.40+axelstra-daemon")).toEqual([0, 1, 40]);
    expect(parseBuild("")).toBeNull();
    expect(parseBuild(null)).toBeNull();
  });
  it("same build is not skew", () => expect(daemonOlderThanApp(base)).toBe(false));
  it("an older release of the daemon is", () => {
    expect(daemonOlderThanApp({ ...base, daemon_build: "0.1.39+axelstra-daemon" })).toBe(true);
    expect(daemonOlderThanApp({ ...base, daemon_build: "0.0.99+x" })).toBe(true);
  });
  it("a newer daemon than the app is not", () =>
    expect(daemonOlderThanApp({ ...base, daemon_build: "0.2.0+x" })).toBe(false));
  it("compares numbers, not strings", () =>
    expect(daemonOlderThanApp({ ...base, app_version: "0.1.9", daemon_build: "0.1.10+x" })).toBe(false));
  it("a daemon without build or with lower proto/security is older", () => {
    expect(daemonOlderThanApp({ ...base, daemon_build: "" })).toBe(true);
    expect(daemonOlderThanApp({ ...base, daemon_proto: 15 })).toBe(true);
    expect(daemonOlderThanApp({ ...base, daemon_security: 1 })).toBe(true);
  });
  it("no daemon, no banner", () =>
    expect(daemonOlderThanApp({ ...base, daemon_connected: false, daemon_build: null })).toBe(false));
});

const diag = (over: Partial<Diagnostics> = {}): Diagnostics => ({
  ...base,
  app_commit: "abcdef1234567",
  daemon_instance: 1,
  app_security_min: 2,
  data_dir: "",
  lag_log: "",
  lag_log_bytes: 0,
  missing_tools: [],
  daemon_version: "0.1.40",
  daemon_commit: "1234567890",
  daemon_offer: null,
  ...over,
});

describe("staleInfo", () => {
  it("same build, no offer: hidden", () => expect(staleInfo(diag()).show).toBe(false));
  it("an offer shows it even when the release numbers match", () => {
    const i = staleInfo(diag({ daemon_offer: { reason: "dev" } }));
    expect(i.show).toBe(true);
    expect(i.reason).toBe("dev");
    expect(i.app).toEqual({ version: "0.1.40", commit: "abcdef1" });
    expect(i.daemon).toEqual({ version: "0.1.40", commit: "1234567" });
  });
  it("an older release shows it without an offer", () => {
    const i = staleInfo(diag({ daemon_build: "0.1.39+axelstra-daemon", daemon_version: "0.1.39" }));
    expect(i.show).toBe(true);
    expect(i.reason).toBeNull();
  });
  it("a daemon that reports no version or commit falls back to its build id, else null", () => {
    const i = staleInfo(diag({ daemon_build: "0.1.39+x", daemon_version: undefined, daemon_commit: null }));
    expect(i.daemon).toEqual({ version: "0.1.39", commit: null });
    const j = staleInfo(diag({ daemon_build: null, daemon_version: null, daemon_commit: "unknown" }));
    expect(j.daemon).toEqual({ version: null, commit: null });
    expect(j.show).toBe(true);
  });
  it("short commits and labels", () => {
    expect(shortCommit(" abcdef1234 ")).toBe("abcdef1");
    expect(shortCommit("")).toBeNull();
    expect(shortCommit(undefined)).toBeNull();
    expect(buildLabel({ version: "0.2.0", commit: "abc1234" }, "?")).toBe("0.2.0 (abc1234)");
    expect(buildLabel({ version: null, commit: null }, "?")).toBe("?");
  });
});
