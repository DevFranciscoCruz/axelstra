import type { TriggerEvent } from "./api";
import { pushNotification } from "./notifications";
import { mayInterrupt } from "./notifyPrefs";
import { toast } from "./toasts";
import { eventText } from "./triggerLogic";

/** One toast and one bell entry per thing the CI-fix trigger did. Returns how many events were handled. */
export const handleTriggerEvents = (events: TriggerEvent[]): number => {
  for (const e of events) {
    const text = eventText(e);
    const kind = e.kind === "ci_fix" ? "info" : "warn";
    // the trigger started a fix by itself: informational. A trigger that could not act is a failure nobody is handling.
    const category = e.kind === "ci_fix" ? "info" : "error";
    if (mayInterrupt(category)) toast({ text, kind, key: `trigger:${e.worktree}:${e.sha ?? e.kind}`, record: false });
    pushNotification({
      kind: "ci-fail",
      worktree: e.worktree,
      text,
      category,
      key: `trigger:${e.worktree}:${e.sha ?? e.kind}`,
    });
  }
  return events.length;
};
