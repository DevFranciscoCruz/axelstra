import { beforeEach, describe, expect, it, vi } from "vitest";

const toast = vi.fn();
const pushNotification = vi.fn();
vi.mock("./toasts", () => ({ toast: (...a: unknown[]) => toast(...a) }));
vi.mock("./notifications", () => ({ pushNotification: (...a: unknown[]) => pushNotification(...a) }));

import { handleTriggerEvents } from "./triggerEvents";
import { DEFAULT_PREFS, setNotifyPref } from "./notifyPrefs";

beforeEach(() => {
  toast.mockReset();
  pushNotification.mockReset();
  for (const k of ["need", "done", "info", "error", "dnd"] as const) setNotifyPref(k, DEFAULT_PREFS[k]);
});

describe("handleTriggerEvents", () => {
  it("a fix the trigger started by itself is informational: recorded, no toast by default", () => {
    handleTriggerEvents([{ kind: "ci_fix", project: "p", worktree: "/wt/a", branch: "ax/a", sha: "abc" }]);
    expect(toast).not.toHaveBeenCalled();
    expect(pushNotification).toHaveBeenCalledWith(expect.objectContaining({ category: "info" }));
  });
  it("toasts and notifies a firing", () => {
    setNotifyPref("info", true);
    expect(handleTriggerEvents([{ kind: "ci_fix", project: "p", worktree: "/wt/a", branch: "ax/a", sha: "abc" }])).toBe(
      1,
    );
    expect(toast).toHaveBeenCalledWith({
      text: "CI falhou em a: a lançar tarefa de correção",
      kind: "info",
      key: "trigger:/wt/a:abc",
      record: false,
    });
    expect(pushNotification).toHaveBeenCalledWith({
      kind: "ci-fail",
      worktree: "/wt/a",
      text: "CI falhou em a: a lançar tarefa de correção",
      category: "info",
      key: "trigger:/wt/a:abc",
    });
  });
  it("a refusal is a warning", () => {
    handleTriggerEvents([{ kind: "ci_fix_limited", project: "p", worktree: "/wt/a", branch: "ax/a", detail: "x" }]);
    expect(toast.mock.calls[0][0].kind).toBe("warn");
  });
  it("nothing to do for no events", () => {
    expect(handleTriggerEvents([])).toBe(0);
    expect(toast).not.toHaveBeenCalled();
  });
});
