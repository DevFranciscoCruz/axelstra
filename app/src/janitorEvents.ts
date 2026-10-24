import type { JanitorEvent } from "./api";
import { pushNotification } from "./notifications";
import { toast } from "./toasts";
import { L } from "./strings.logic";

export const janitorText = (e: Pick<JanitorEvent, "branch" | "into">): string =>
  L.notify.janitor(e.branch.replace(/^ax\//, ""), e.into);

/** One toast and one bell entry per removed worktree; both dedupe by path. Returns how many events were handled. */
export const handleJanitorEvents = (events: JanitorEvent[]): number => {
  for (const e of events) {
    const text = janitorText(e);
    toast({ text, kind: "info", key: `janitor:${e.path}` });
    pushNotification({ kind: "merged", worktree: e.path, text });
  }
  return events.length;
};
