import { beforeEach, describe, expect, it, vi } from "vitest";

const toast = vi.fn();
const pushNotification = vi.fn();
vi.mock("./toasts", () => ({ toast: (...a: unknown[]) => toast(...a) }));
vi.mock("./notifications", () => ({ pushNotification: (...a: unknown[]) => pushNotification(...a) }));

import { handleJanitorEvents, janitorText } from "./janitorEvents";
import { DEFAULT_PREFS, setNotifyPref } from "./notifyPrefs";

const ev = (branch: string, into = "main", path = `/wt/${branch}`) => ({
  project: "p",
  path,
  branch,
  into,
  why: "merged",
  at: 1,
});

beforeEach(() => {
  toast.mockReset();
  pushNotification.mockReset();
  for (const k of ["need", "done", "info", "error", "dnd"] as const) setNotifyPref(k, DEFAULT_PREFS[k]);
});

describe("janitorText", () => {
  it("drops the ax/ prefix", () => {
    expect(janitorText(ev("ax/foo"))).toBe("worktree foo removido (integrado em main)");
  });
  it("keeps a branch without the prefix and names the target", () => {
    expect(janitorText(ev("bar", "ax/baz"))).toBe("worktree bar removido (integrado em ax/baz)");
  });
});

describe("handleJanitorEvents", () => {
  it("is silent by default: removals reach the centre but raise no toast", () => {
    expect(handleJanitorEvents([ev("ax/a")])).toBe(1);
    expect(toast).not.toHaveBeenCalled();
    expect(pushNotification).toHaveBeenCalledTimes(1);
  });
  it("one event gets one toast keyed by path, a batch gets a single aggregate toast", () => {
    setNotifyPref("info", true);
    handleJanitorEvents([ev("ax/a")]);
    expect(toast).toHaveBeenCalledWith({
      text: "worktree a removido (integrado em main)",
      kind: "info",
      key: "janitor:/wt/ax/a",
    });
    toast.mockReset();
    pushNotification.mockReset();
    expect(handleJanitorEvents([ev("ax/a"), ev("ax/b", "ax/a")])).toBe(2);
    expect(toast).toHaveBeenCalledTimes(1);
    expect(toast).toHaveBeenCalledWith({
      text: "2 worktrees integrados e removidos",
      kind: "info",
      key: "janitor:batch",
    });
    expect(pushNotification).toHaveBeenNthCalledWith(2, {
      kind: "merged",
      worktree: "/wt/ax/b",
      text: "worktree b removido (integrado em ax/a)",
    });
  });
  it("does nothing for an empty list", () => {
    expect(handleJanitorEvents([])).toBe(0);
    expect(toast).not.toHaveBeenCalled();
    expect(pushNotification).not.toHaveBeenCalled();
  });
  it("integrated and empty events get their own text", () => {
    setNotifyPref("info", true);
    handleJanitorEvents([
      { ...ev("ax/a"), kind: "integrated", how: "ff", pushed: null },
      { ...ev("ax/b"), kind: "empty" },
    ]);
    expect(pushNotification).toHaveBeenNthCalledWith(2, {
      kind: "merged",
      worktree: "/wt/ax/b",
      text: "Worktree vazio b removido",
    });
  });
  it("do not disturb keeps the entry but raises no toast, even for an attention", () => {
    setNotifyPref("dnd", true);
    handleJanitorEvents([{ ...ev("ax/c"), kind: "attention", code: "integrate.busy", params: {}, text: "x" }]);
    expect(toast).not.toHaveBeenCalled();
    expect(pushNotification).toHaveBeenCalledTimes(1);
  });
  it("attention shows the coded text as a warning keyed by path and code", () => {
    handleJanitorEvents([
      { ...ev("ax/c"), kind: "attention", code: "integrate.busy", params: { branch: "ax/c" }, text: "english" },
      { ...ev("ax/d"), kind: "attention", code: "unknown.code", text: "english fallback" },
    ]);
    expect(toast).toHaveBeenNthCalledWith(
      1,
      expect.objectContaining({
        kind: "warn",
        key: "janitor:/wt/ax/c:integrate.busy",
        text: expect.stringContaining("ax/c"),
      }),
    );
    expect(pushNotification).toHaveBeenNthCalledWith(
      1,
      expect.objectContaining({ kind: "attention", worktree: "/wt/ax/c" }),
    );
    expect(toast).toHaveBeenNthCalledWith(2, expect.objectContaining({ text: "english fallback" }));
  });
  it("reclaimed builds get a disk entry each and one info toast with the total, apart from the removals", () => {
    setNotifyPref("info", true);
    handleJanitorEvents([
      { ...ev("ax/a"), kind: "artifacts", freed_mb: 1024 },
      { ...ev("ax/b"), kind: "artifacts", freed_mb: 8192 },
    ]);
    expect(pushNotification).toHaveBeenNthCalledWith(1, {
      kind: "disk",
      worktree: "/wt/ax/a",
      text: "Builds de a apagados: 1,0 GB libertados",
    });
    expect(toast).toHaveBeenCalledTimes(1);
    expect(toast).toHaveBeenCalledWith({
      text: "Builds de 2 worktrees inativos apagados: 9,0 GB libertados",
      kind: "info",
      key: "janitor:artifacts",
    });
  });
  it("a pending removal names the holder, or shows the daemon's coded reason", () => {
    handleJanitorEvents([{ ...ev("ax/p"), kind: "pending", holder: { name: "Code.exe", pid: 41 } }]);
    expect(toast).toHaveBeenCalledWith({
      text: "Worktree p não foi removido: a pasta está presa por Code.exe (pid 41)",
      kind: "warn",
      key: "janitor:/wt/ax/p:pending",
    });
    expect(pushNotification).toHaveBeenCalledWith(expect.objectContaining({ kind: "attention", worktree: "/wt/ax/p" }));
    toast.mockReset();
    handleJanitorEvents([
      { ...ev("ax/q"), kind: "pending", code: "artifacts.busy", params: { path: "/wt/ax/q" }, text: "english" },
    ]);
    expect(toast).toHaveBeenCalledWith(expect.objectContaining({ text: expect.stringContaining("está em uso") }));
  });
});
