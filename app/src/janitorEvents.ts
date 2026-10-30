import type { JanitorEvent } from "./api";
import { coded } from "./msg";
import { pushNotification } from "./notifications";
import { mayInterrupt } from "./notifyPrefs";
import { toast } from "./toasts";
import { L } from "./strings.logic";
import { D } from "./strings.disk";
import { fmtMb } from "./diskLogic";

const short = (branch: string) => branch.replace(/^ax\//, "");

export const janitorText = (e: Pick<JanitorEvent, "branch" | "into">): string =>
  L.notify.janitor(short(e.branch), e.into);

/** "Code.exe (pid 412)" of a pending removal's holder, empty when the daemon did not know it */
const holderOf = (e: JanitorEvent): string => {
  const h = e.holder;
  if (!h) return "";
  if (typeof h === "string") return h;
  return h.name ? (h.pid ? `${h.name} (pid ${h.pid})` : h.name) : h.pid ? `pid ${h.pid}` : "";
};

/** Builds the janitor reclaimed: one bell entry each (kind `disk`) and one info toast for the batch, when info may interrupt. */
const handleArtifacts = (events: JanitorEvent[]) => {
  if (!events.length) return;
  const total = events.reduce((n, e) => n + (e.freed_mb ?? 0), 0);
  for (const e of events)
    pushNotification({
      kind: "disk",
      worktree: e.path,
      text: D.notify.artifacts(short(e.branch), fmtMb(e.freed_mb ?? 0)),
    });
  if (mayInterrupt("info"))
    toast({
      text:
        events.length > 1
          ? D.notify.artifactsAgg(events.length, fmtMb(total))
          : D.notify.artifacts(short(events[0].branch), fmtMb(total)),
      kind: "info",
      key: "janitor:artifacts",
    });
};

/** One bell entry per event (removals fold into "N worktrees integrados"); a toast for an attention (needs a person) and
 *  one toast for all the removals of a batch, only when their category may interrupt. Returns how many events were handled. */
export const handleJanitorEvents = (events: JanitorEvent[]): number => {
  handleArtifacts(events.filter((e) => e.kind === "artifacts"));
  const removed = events.filter((e) => e.kind !== "attention" && e.kind !== "pending" && e.kind !== "artifacts").length;
  let toasted = false;
  for (const e of events) {
    if (e.kind === "artifacts") continue;
    if (e.kind === "pending") {
      const text = e.code ? coded(e.code, e.params, e.text ?? e.code) : D.notify.pending(short(e.branch), holderOf(e));
      if (mayInterrupt("need")) toast({ text, kind: "warn", key: `janitor:${e.path}:pending` });
      pushNotification({ kind: "attention", worktree: e.path, text });
      continue;
    }
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
