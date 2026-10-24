import { beforeEach, describe, expect, it, vi } from "vitest";

const toast = vi.fn();
const pushNotification = vi.fn();
vi.mock("./toasts", () => ({ toast: (...a: unknown[]) => toast(...a) }));
vi.mock("./notifications", () => ({ pushNotification: (...a: unknown[]) => pushNotification(...a) }));

import { handleTriggerEvents } from "./triggerEvents";

beforeEach(() => {
  toast.mockReset();
  pushNotification.mockReset();
});

describe("handleTriggerEvents", () => {
  it("toasts and notifies a firing", () => {
    expect(handleTriggerEvents([{ kind: "ci_fix", project: "p", worktree: "/wt/a", branch: "ax/a", sha: "abc" }])).toBe(
      1,
    );
    expect(toast).toHaveBeenCalledWith({
      text: "CI falhou em a: a lançar tarefa de correção",
      kind: "info",
      key: "trigger:/wt/a:abc",
    });
    expect(pushNotification).toHaveBeenCalledWith({
      kind: "ci-fail",
      worktree: "/wt/a",
      text: "CI falhou em a: a lançar tarefa de correção",
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
