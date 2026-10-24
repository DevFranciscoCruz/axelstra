import type { TriggerEvent } from "./api";
import { pushNotification } from "./notifications";
import { toast } from "./toasts";
import { eventText } from "./triggerLogic";

/** One toast and one bell entry per thing the CI-fix trigger did. Returns how many events were handled. */
export const handleTriggerEvents = (events: TriggerEvent[]): number => {
  for (const e of events) {
    const text = eventText(e);
    const kind = e.kind === "ci_fix" ? "info" : "warn";
    toast({ text, kind, key: `trigger:${e.worktree}:${e.sha ?? e.kind}` });
    pushNotification({ kind: "ci-fail", worktree: e.worktree, text });
  }
  return events.length;
};
