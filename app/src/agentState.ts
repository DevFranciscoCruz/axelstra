import type { SessionInfo } from "./api";
import { sessionState } from "./store";
import { L } from "./strings.logic";

/** What a status dot shows besides its colour: a shape, and the words for the tooltip and screen readers. */
export type DotState = "working" | "idle" | "waiting" | "done" | "attention" | "queued" | "running" | "none";

export const DOT_LABEL: Record<DotState, string> = L.dot;

/** Glyph paths (8x8 box, stroked): a different silhouette per state, so colour is never the only cue. */
export const DOT_GLYPH: Record<DotState, string> = {
  working: "M4 1.3a2.7 2.7 0 1 1-2.7 2.7",
  idle: "M3 2.2v3.6M5 2.2v3.6",
  waiting: "M2.9 3.1a1.1 1.1 0 1 1 1.6 1c-.4.3-.5.5-.5 1M4 6.3v.01",
  done: "M1.9 4.2l1.5 1.5 2.7-3.3",
  attention: "M4 1.6v3M4 6.2v.01",
  queued: "M4 1.8v2.4l1.3.9",
  running: "M2 2.5l2 1.5-2 1.5M4.8 6h1.4",
  none: "",
};

/** State of one session for its dot: agents say what they are doing, a plain terminal only whether it runs. */
export const dotOfSession = (s: SessionInfo): DotState => {
  const st = sessionState(s);
  if (s.meta?.kind !== "agent") return s.running ? "running" : "done";
  return st === "waiting" ? "waiting" : st === "working" ? "working" : st === "done" ? "done" : "idle";
};

/** State of a worktree row from what its agents are doing, most urgent first. */
export const dotOfWorktree = (o: { queued: boolean; waiting: boolean; working: boolean; needs: boolean; live: number }): DotState =>
  o.queued ? "queued" : o.waiting ? "waiting" : o.working ? "working" : o.needs ? "attention" : o.live > 0 ? "idle" : "none";
