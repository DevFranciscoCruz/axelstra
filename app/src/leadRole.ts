import type { SessionInfo } from "./api";

/** The lead of a worktree is its first agent ("main"): it owns the worktree's theme and the tasks started there report to it. */
export const isLead = (s: Pick<SessionInfo, "meta">) => s.meta?.lead === true;

/** The agents of a worktree with the lead first and the tasks after it, each group in its original order. */
export const leadFirst = <T extends Pick<SessionInfo, "meta">>(agents: T[]): T[] => [
  ...agents.filter(isLead),
  ...agents.filter((a) => !isLead(a)),
];

/** A worktree (never the project's main checkout) with no running lead offers to start one. */
export const lacksLead = (agents: Pick<SessionInfo, "meta" | "running">[], isMain: boolean) =>
  !isMain && !agents.some((a) => a.running && isLead(a));

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
