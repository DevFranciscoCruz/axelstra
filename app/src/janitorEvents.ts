import type { JanitorEvent } from "./api";
import { coded } from "./msg";
import { pushNotification } from "./notifications";
import { toast } from "./toasts";
import { L } from "./strings.logic";

const short = (branch: string) => branch.replace(/^ax\//, "");

export const janitorText = (e: Pick<JanitorEvent, "branch" | "into">): string =>
  L.notify.janitor(short(e.branch), e.into);

/** One toast and one bell entry per event; both dedupe by path (attention: by path and code). Returns how many events were handled. */
export const handleJanitorEvents = (events: JanitorEvent[]): number => {
  for (const e of events) {
    if (e.kind === "attention") {
      const text = coded(e.code, e.params, e.text ?? e.code ?? "");
      toast({ text, kind: "warn", key: `janitor:${e.path}:${e.code}` });
      pushNotification({ kind: "attention", worktree: e.path, text });
      continue;
    }
    const text =
      e.kind === "integrated"
        ? L.notify.integrated(short(e.branch), e.into)
        : e.kind === "empty"
          ? L.notify.emptyRemoved(short(e.branch))
          : janitorText(e);
    toast({ text, kind: "info", key: `janitor:${e.path}` });
    pushNotification({ kind: "merged", worktree: e.path, text });
  }
  return events.length;
};
