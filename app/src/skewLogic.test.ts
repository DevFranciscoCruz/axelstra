import { describe, expect, it } from "vitest";
import { daemonOlderThanApp, parseBuild } from "./skewLogic";

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
