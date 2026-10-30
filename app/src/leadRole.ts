import type { SessionInfo } from "./api";
import { norm } from "./paths";

/** A lead ("main" of a worktree) exists per request, only when asked for (`meta.lead`): it owns that theme and the tasks
 *  started in its worktree report to it. A worktree with plain agents and no lead is normal. */
export const isLead = (s: Pick<SessionInfo, "meta">) => s.meta?.lead === true;

/** The agents of a worktree with the lead first and the tasks after it, each group in its original order. */
export const leadFirst = <T extends Pick<SessionInfo, "meta">>(agents: T[]): T[] => [
  ...agents.filter(isLead),
  ...agents.filter((a) => !isLead(a)),
];

/** Starting a lead is an optional action, offered in a worktree (never the project's main checkout) with no running lead.
 *  Not having one is not a problem and is never flagged. */
export const canStartLead = (agents: Pick<SessionInfo, "meta" | "running">[], isMain: boolean) =>
  !isMain && !agents.some((a) => a.running && isLead(a));

/** The running lead of the worktree at `path` in a full session list (any state: idle, working, waiting), or undefined.
 *  Paths compare in their normal form, so slash style, case and a trailing slash do not hide a lead. */
export const leadOf = (all: SessionInfo[], path: string) =>
  all.find((s) => s.running && s.meta?.kind === "agent" && isLead(s) && norm(s.meta?.worktree) === norm(path));

/** What the state dot of a worktree row reads from its agents. With a running lead the lead speaks for the worktree
 *  (working / idle), and a task only adds "waiting" because someone has to answer it; without a lead every agent counts. */
export const worktreeSignals = <
  T extends Pick<SessionInfo, "meta" | "running"> & { stalled?: boolean; awaiting?: unknown },
>(
  sessions: T[],
  stateOf: (s: T) => "working" | "waiting" | "idle" | "done",
) => {
  const running = sessions.filter((s) => s.running);
  const lead = running.find((s) => s.meta?.kind === "agent" && isLead(s));
  const waiting = running.some((s) => stateOf(s) === "waiting");
  const working = lead ? stateOf(lead) === "working" : running.some((s) => stateOf(s) === "working");
  // an agent that went quiet without reporting: any session of the worktree counts, the lead or a task
  const stalled = running.some((s) => s.meta?.kind === "agent" && s.stalled === true && !s.awaiting);
  // an agent that only waits for something external (CI, deploy): not working, not stalled, not a question
  const awaiting = running.some((s) => s.meta?.kind === "agent" && !!s.awaiting);
  return { live: running.length, waiting, working, stalled, awaiting };
};

/** The worktree's own "main": its lead, or the project's main session in the main checkout (`meta.main`, or the row the
 *  daemon named "main"). Never a worker. */
export const isMainSeat = (s: Pick<SessionInfo, "meta" | "label">) =>
  s.meta?.worker !== true && (isLead(s) || s.meta?.main === true || s.label === "main");

/** The session a freshly selected worktree opens on: its main (running first), else a terminal (running first). Never a
 *  plain agent or worker ("Tarefa N"): with only tasks the seat stays empty and the user picks one. */
export const autoSeatOf = <T extends Pick<SessionInfo, "meta" | "label" | "running">>(tabs: T[]): T | undefined => {
  const live = (list: T[]) => list.find((s) => s.running) ?? list[0];
  return (
    live(tabs.filter((s) => s.meta?.kind === "agent" && isMainSeat(s))) ??
    live(tabs.filter((s) => s.meta?.kind !== "agent"))
  );
};
