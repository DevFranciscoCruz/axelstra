import { describe, expect, it } from "vitest";
import type { HostInfo } from "./api";
import {
  applyHostState,
  breadcrumbs,
  canReconnect,
  classifyRemoteError,
  isAbsRemote,
  joinRemote,
  moveRow,
  pathProblem,
  sepOf,
  specProblem,
} from "./hostLogic";

describe("specProblem", () => {
  it("accepts the documented forms", () => {
    for (const s of [
      "wsl:Ubuntu-22.04",
      "ssh:lab",
      "ssh:me@lab.example.com",
      "ssh:me@lab:2222",
      "ssh:10.0.0.5:22",
      "ssh:me@[::1]:22",
      "ssh:[fe80::1]",
    ])
      expect(specProblem(s), s).toBeNull();
  });
  it("rejects empty, unknown scheme and whitespace", () => {
    expect(specProblem("")).toBe("empty");
    expect(specProblem("   ")).toBe("empty");
    expect(specProblem("lab")).toBe("scheme");
    expect(specProblem("telnet:lab")).toBe("scheme");
    expect(specProblem("ssh:my lab")).toBe("space");
    expect(specProblem("ssh:lab\nfoo")).toBe("space");
  });
  it("never lets an option through", () => {
    expect(specProblem("ssh:-oProxyCommand=x")).toBe("dash");
    expect(specProblem("ssh:-l@lab")).toBe("dash");
    expect(specProblem("ssh:lab:-1")).toBe("dash");
    expect(specProblem("wsl:-d")).toBe("dash");
    expect(specProblem("ssh:lab -o x")).toBe("space");
  });
  it("checks every part", () => {
    expect(specProblem("wsl:")).toBe("distro");
    expect(specProblem("wsl:a/b")).toBe("distro");
    expect(specProblem("ssh:")).toBe("host");
    expect(specProblem("ssh:@lab")).toBe("user");
    expect(specProblem("ssh:me@")).toBe("host");
    expect(specProblem("ssh:lab:0")).toBe("port");
    expect(specProblem("ssh:lab:70000")).toBe("port");
    expect(specProblem("ssh:lab:")).toBe("port");
    expect(specProblem("ssh:[::1")).toBe("host");
    expect(specProblem("ssh:la$b")).toBe("host");
  });
});

const host = (over: Partial<HostInfo> = {}): HostInfo => ({
  id: "lab",
  name: "Lab",
  spec: "ssh:lab",
  slot: 1,
  state: "idle",
  ...over,
});

describe("applyHostState", () => {
  it("moves a host through its states and keeps the error only while not ok", () => {
    let l = [host(), host({ id: "other" })];
    l = applyHostState(l, { id: "lab", state: "connecting" });
    expect(l[0].state).toBe("connecting");
    l = applyHostState(l, { id: "lab", state: "auth", error: "Permission denied (publickey)" });
    expect(l[0]).toMatchObject({ state: "auth", error: "Permission denied (publickey)" });
    l = applyHostState(l, { id: "lab", state: "ok", error: "stale" });
    expect(l[0].error).toBeUndefined();
    expect(l[1].state).toBe("idle");
  });
  it("ignores an unknown host", () => {
    const l = [host()];
    expect(applyHostState(l, { id: "nope", state: "ok" })).toBe(l);
  });
  it("offers reconnect only when the link is not up or coming up", () => {
    expect(["lost", "auth", "idle"].every((s) => canReconnect(s as never))).toBe(true);
    expect(canReconnect("ok")).toBe(false);
    expect(canReconnect("connecting")).toBe(false);
    expect(canReconnect(undefined)).toBe(false);
  });
});

describe("classifyRemoteError", () => {
  it("tells the failures apart", () => {
    expect(classifyRemoteError("me@lab: Permission denied (publickey).")).toBe("auth");
    expect(classifyRemoteError("bash: axelstra: command not found")).toBe("no-axelstra");
    expect(classifyRemoteError("daemon not reachable on this machine; start it there")).toBe("no-daemon");
    expect(classifyRemoteError("ssh: Could not resolve hostname lab")).toBe("unreachable");
    expect(classifyRemoteError("Connection timed out")).toBe("unreachable");
    expect(classifyRemoteError("something odd")).toBe("other");
  });
});

describe("breadcrumbs", () => {
  it("splits a unix path from the root", () => {
    expect(breadcrumbs("/home/me/proj")).toEqual([
      { label: "/", path: "/" },
      { label: "home", path: "/home" },
      { label: "me", path: "/home/me" },
      { label: "proj", path: "/home/me/proj" },
    ]);
    expect(breadcrumbs("/")).toEqual([{ label: "/", path: "/" }]);
    expect(breadcrumbs("")).toEqual([]);
  });
  it("starts a Windows remote at its drive", () => {
    expect(breadcrumbs(String.raw`C:\Users\me`)).toEqual([
      { label: "C:\\", path: "C:\\" },
      { label: "Users", path: String.raw`C:\Users` },
      { label: "me", path: String.raw`C:\Users\me` },
    ]);
  });
  it("joins with the remote's separator", () => {
    expect(joinRemote("/home/me", "x")).toBe("/home/me/x");
    expect(joinRemote("/", "x")).toBe("/x");
    expect(joinRemote("C:\\", "x", "\\")).toBe(String.raw`C:\x`);
  });
  it("tells absolute remote paths", () => {
    expect(isAbsRemote("/home/me")).toBe(true);
    expect(isAbsRemote(String.raw`D:\work`)).toBe(true);
    expect(isAbsRemote("home/me")).toBe(false);
    expect(isAbsRemote("~/x")).toBe(false);
    expect(isAbsRemote("  ")).toBe(false);
  });
});

describe("browse helpers", () => {
  it("validates the typed path", () => {
    expect(pathProblem("")).toBe("empty");
    expect(pathProblem("  ")).toBe("empty");
    expect(pathProblem("proj")).toBe("relative");
    expect(pathProblem("/home/me/proj")).toBeNull();
    expect(pathProblem("D:\\work")).toBeNull();
  });
  it("picks the separator of the remote", () => {
    expect(sepOf("/home/me")).toBe("/");
    expect(sepOf("C:\\Users")).toBe("\\");
    expect(sepOf("")).toBe("/");
  });
  it("moves the highlighted row and wraps", () => {
    expect(moveRow(-1, 3, "ArrowDown")).toBe(0);
    expect(moveRow(2, 3, "ArrowDown")).toBe(0);
    expect(moveRow(-1, 3, "ArrowUp")).toBe(2);
    expect(moveRow(0, 3, "ArrowUp")).toBe(2);
    expect(moveRow(1, 3, "End")).toBe(2);
    expect(moveRow(1, 3, "Home")).toBe(0);
    expect(moveRow(1, 3, "x")).toBe(1);
    expect(moveRow(0, 0, "ArrowDown")).toBe(-1);
  });
});
