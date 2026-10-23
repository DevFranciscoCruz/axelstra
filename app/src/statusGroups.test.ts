import { describe, expect, it } from "vitest";
import type { SessionInfo } from "./api";
import { countsFor, groupAgents, mergedList, sumCounts, type OverviewProject } from "./statusGroups";

const sess = (id: number, over: Partial<SessionInfo> = {}, wt = "C:/w/feat-a"): SessionInfo => ({ id, running: true, label: null, meta: { kind: "agent", worktree: wt }, ...over }) as SessionInfo;
const WIN = ["C:", "w", "fix-b", ""].join(String.fromCharCode(92));
const stateOf = (s: SessionInfo) => (!s.running ? "done" : s.waiting ? "waiting" : (s.idle_ms ?? Infinity) < 3000 ? "working" : "idle");

describe("agent groups", () => {
  it("splits running, idle and waiting agents and skips terminals and finished ones", () => {
    const list = [
      sess(3, { waiting: true }, WIN),
      sess(1, { idle_ms: 100 }),
      sess(2, { idle_ms: 90000 }),
      sess(4, { running: false }),
      sess(5, { meta: { kind: "shell", worktree: "C:/w/x" } } as Partial<SessionInfo>),
    ];
    const g = groupAgents(list, stateOf, (s) => `Agente ${s.id}`);
    expect(g.running.map((e) => e.id)).toEqual([1]);
    expect(g.idle.map((e) => e.id)).toEqual([2]);
    expect(g.waiting).toEqual([{ id: 3, name: "Agente 3", worktree: "fix-b", path: WIN }]);
    expect(g.running[0].worktree).toBe("feat-a");
  });
});

const proj = (id: string, total: number, merged: number): OverviewProject => ({
  id,
  name: id.toUpperCase(),
  counts: { total, dirty: 1, ahead: 0, behind: 2, merged, no_agent: total },
  merged: Array.from({ length: merged }, (_, i) => ({ path: `/w/${id}${i}`, branch: `ax/${id}${i}`, why: "already in origin/main" })),
});

describe("worktree overview", () => {
  it("sums over projects and picks the current one", () => {
    const ps = [proj("a", 3, 1), proj("b", 2, 2)];
    expect(sumCounts(ps)).toEqual({ total: 5, dirty: 2, ahead: 0, behind: 4, merged: 3, no_agent: 5 });
    const c = countsFor(ps, "b");
    expect(c.current.total).toBe(2);
    expect(c.all.total).toBe(5);
    expect(countsFor(ps, "zzz").current.total).toBe(0);
    expect(countsFor([], null).all.total).toBe(0);
  });
  it("lists merged worktrees of every project with the project name", () => {
    const l = mergedList([proj("a", 3, 1), proj("b", 2, 2)]);
    expect(l.map((m) => `${m.project}:${m.branch}`)).toEqual(["A:ax/a0", "B:ax/b0", "B:ax/b1"]);
  });
});
