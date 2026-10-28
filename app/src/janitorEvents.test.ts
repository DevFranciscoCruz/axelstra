import { beforeEach, describe, expect, it, vi } from "vitest";

const toast = vi.fn();
const pushNotification = vi.fn();
vi.mock("./toasts", () => ({ toast: (...a: unknown[]) => toast(...a) }));
vi.mock("./notifications", () => ({ pushNotification: (...a: unknown[]) => pushNotification(...a) }));

import { handleJanitorEvents, janitorText } from "./janitorEvents";

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
  it("toasts and notifies once per event, keyed by path", () => {
    expect(handleJanitorEvents([ev("ax/a"), ev("ax/b", "ax/a")])).toBe(2);
    expect(toast).toHaveBeenCalledTimes(2);
    expect(toast).toHaveBeenNthCalledWith(1, {
      text: "worktree a removido (integrado em main)",
      kind: "info",
      key: "janitor:/wt/ax/a",
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
  it("integrated and empty events get their own text and info toast", () => {
    handleJanitorEvents([
      { ...ev("ax/a"), kind: "integrated", how: "ff", pushed: null },
      { ...ev("ax/b"), kind: "empty" },
    ]);
    expect(toast).toHaveBeenNthCalledWith(1, {
      text: "worktree a integrado em main e removido",
      kind: "info",
      key: "janitor:/wt/ax/a",
    });
    expect(pushNotification).toHaveBeenNthCalledWith(2, {
      kind: "merged",
      worktree: "/wt/ax/b",
      text: "Worktree vazio b removido",
    });
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
});
