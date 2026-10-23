import type { SessionInfo } from "./api";

/** One agent in the status bar's popover. */
export type AgentEntry = { id: number; name: string; worktree: string; path: string };
export type AgentGroups = { running: AgentEntry[]; idle: AgentEntry[]; waiting: AgentEntry[] };

type State = "working" | "waiting" | "idle" | "done";

const last = (p: string) => p.split(/[\\/]+/).filter(Boolean).pop() ?? p;

/** Running agents (not plain terminals, not finished ones) split by what they are doing, in session order. */
export const groupAgents = (list: SessionInfo[], stateOf: (s: SessionInfo) => State, nameOf: (s: SessionInfo) => string): AgentGroups => {
  const g: AgentGroups = { running: [], idle: [], waiting: [] };
  for (const s of [...list].sort((a, b) => a.id - b.id)) {
    if (s.meta?.kind !== "agent") continue;
    const st = stateOf(s);
    if (st === "done") continue;
    const path = s.meta?.worktree ?? "";
    const e = { id: s.id, name: nameOf(s), worktree: path ? last(path) : "", path };
    (st === "working" ? g.running : st === "waiting" ? g.waiting : g.idle).push(e);
  }
  return g;
};

/** What `worktree.overview` returns per project. */
export type WtCounts = { total: number; dirty: number; ahead: number; behind: number; merged: number; no_agent: number };
export type MergedWt = { path: string; branch: string; why: string };
export type OverviewProject = { id: string; name: string; counts: WtCounts; merged: MergedWt[] };

const ZERO: WtCounts = { total: 0, dirty: 0, ahead: 0, behind: 0, merged: 0, no_agent: 0 };

export const sumCounts = (projects: OverviewProject[]): WtCounts =>
  projects.reduce((a, p) => ({ total: a.total + p.counts.total, dirty: a.dirty + p.counts.dirty, ahead: a.ahead + p.counts.ahead, behind: a.behind + p.counts.behind, merged: a.merged + p.counts.merged, no_agent: a.no_agent + p.counts.no_agent }), ZERO);

/** The counts of one project (all zero when the daemon does not know it) next to the sum over all projects. */
export const countsFor = (projects: OverviewProject[], current: string | null | undefined) => ({
  all: sumCounts(projects),
  current: projects.find((p) => p.id === current)?.counts ?? ZERO,
});

/** The "Limpar merged" list: the sweep acts on every project, so the list shows the merged worktrees of all of them. */
export const mergedList = (projects: OverviewProject[]) => projects.flatMap((p) => p.merged.map((m) => ({ ...m, project: p.name })));
