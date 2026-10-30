import type { SessionInfo } from "../api";
import { isLead, isMainSeat, leadFirst } from "../leadRole";
import { norm } from "../paths";
import { P } from "../strings.panels";

type Agent = Pick<SessionInfo, "id" | "meta" | "label" | "running">;

/** How the agents under one worktree row are shown. A worktree: its lead first (`seats`), its tasks indented after it.
 *  The project's main checkout has no tasks (main only dispatches): its own session is the seat, and any other agent that
 *  already sits there from before the rule is `legacy`, shown apart and muted, never killed or moved. */
export const agentGroups = <T extends Agent>(agents: T[], isMain: boolean) => {
  if (isMain)
    return {
      seats: agents.filter(isMainSeat),
      tasks: [] as T[],
      legacy: agents.filter((a) => !isMainSeat(a)),
    };
  const ordered = leadFirst(agents);
  return { seats: ordered.filter(isLead), tasks: ordered.filter((a) => !isLead(a)), legacy: [] as T[] };
};

/** The worktree row shows as selected only while the worktree itself or its main is in view: with a task (or a legacy task
 *  of the main checkout) active, that task's row is the selected one, not the worktree above it. */
export const worktreeRowActive = (
  selected: string | null | undefined,
  path: string,
  activeId: number | null,
  sessions: Agent[],
) => {
  if (norm(selected) !== norm(path)) return false;
  const a = activeId === null ? undefined : sessions.find((s) => s.id === activeId);
  return !a || a.meta?.kind !== "agent" || isMainSeat(a);
};

/** What "new agent" does in a worktree. In the main checkout the agent the user starts IS the project's main, and there is
 *  only one: with a main already running, a second agent there would be a task, so the user is sent to a new worktree. */
export const manualAgentPlan = (wt: { is_main: boolean }, sessions: Agent[]): "spawn" | "worktree" =>
  wt.is_main && sessions.some((s) => s.running && s.meta?.kind === "agent" && isMainSeat(s)) ? "worktree" : "spawn";

/** Runs before the app starts an agent the user asked for. False (and a notice offering a new worktree) when the agent would
 *  be a second one in the main checkout; the caller then starts nothing. */
export const allowManualAgent = (
  c: { project: { id: string }; wt: { path: string; is_main: boolean } },
  sessions: Agent[],
  deps: {
    notify: (text: string, action: { label: string; run: () => void }) => void;
    newWorktree: (project: string) => void;
  },
) => {
  if (manualAgentPlan(c.wt, sessions) === "spawn") return true;
  deps.notify(P.sidebar.mainBusy, { label: P.sidebar.newWorktree, run: () => deps.newWorktree(c.project.id) });
  return false;
};
