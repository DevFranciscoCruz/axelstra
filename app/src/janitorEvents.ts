import type { JanitorEvent } from "./api";
import { coded } from "./msg";
import { pushNotification } from "./notifications";
import { mayInterrupt } from "./notifyPrefs";
import { toast } from "./toasts";
import { L } from "./strings.logic";

const short = (branch: string) => branch.replace(/^ax\//, "");

export const janitorText = (e: Pick<JanitorEvent, "branch" | "into">): string =>
  L.notify.janitor(short(e.branch), e.into);

/** One bell entry per event (removals fold into "N worktrees integrados"); a toast for an attention (needs a person) and
 *  one toast for all the removals of a batch, only when their category may interrupt. Returns how many events were handled. */
export const handleJanitorEvents = (events: JanitorEvent[]): number => {
  const removed = events.filter((e) => e.kind !== "attention").length;
  let toasted = false;
  for (const e of events) {
    if (e.kind === "attention") {
      const text = coded(e.code, e.params, e.text ?? e.code ?? "");
      if (mayInterrupt("need")) toast({ text, kind: "warn", key: `janitor:${e.path}:${e.code}` });
      pushNotification({ kind: "attention", worktree: e.path, text });
      continue;
    }
    const text = e.code
      ? coded(e.code, e.params, e.text ?? e.code)
      : e.kind === "integrated"
        ? L.notify.integrated(short(e.branch), e.into)
        : e.kind === "empty"
          ? L.notify.emptyRemoved(short(e.branch))
          : janitorText(e);
    if (mayInterrupt("info") && !toasted) {
      toasted = true;
      toast({
        text: removed > 1 ? L.notify.agg.merged(removed) : text,
        kind: "info",
        key: removed > 1 ? "janitor:batch" : `janitor:${e.path}`,
      });
    }
    pushNotification({ kind: "merged", worktree: e.path, text });
  }
  return events.length;
};
