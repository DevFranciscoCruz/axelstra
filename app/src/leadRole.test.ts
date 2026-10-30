import { describe, expect, it } from "vitest";
import type { SessionMeta } from "./api";
import { autoSeatOf, canStartLead, isLead, leadFirst, leadOf, worktreeSignals } from "./leadRole";
import { agentRowInfo } from "./agentRow";

const a = (id: number, over: { lead?: boolean; running?: boolean; state?: string; kind?: string } = {}) => ({
  id,
  label: over.lead ? "main" : `Tarefa ${id}`,
  running: over.running ?? true,
  state: over.state ?? "idle",
  meta: { kind: over.kind ?? "agent", ...(over.lead ? { lead: true } : {}) },
});
const stateOf = (s: { state: string }) => s.state as "working" | "waiting" | "idle" | "done";

describe("lead of a worktree in the sidebar", () => {
  it("lists the lead first and keeps the tasks in their order", () => {
    const list = [a(1), a(2), a(3, { lead: true }), a(4)];
    expect(leadFirst(list).map((s) => s.id)).toEqual([3, 1, 2, 4]);
    expect(leadFirst([a(1), a(2)]).map((s) => s.id)).toEqual([1, 2]);
  });
  it("marks the lead row", () => {
    expect(isLead(a(1, { lead: true }))).toBe(true);
    expect(isLead(a(2))).toBe(false);
    expect(agentRowInfo(a(1, { lead: true }), "C:/w").lead).toBe(true);
    expect(agentRowInfo(a(2), "C:/w").lead).toBe(false);
  });
  it("offers to start a lead (optional) only in a worktree that has none running, never in the main checkout", () => {
    expect(canStartLead([], false)).toBe(true);
    expect(canStartLead([a(1)], false)).toBe(true);
    expect(canStartLead([a(1, { lead: true, running: false })], false)).toBe(true);
    expect(canStartLead([a(1, { lead: true })], false)).toBe(false);
    expect(canStartLead([], true)).toBe(false);
  });
  it("a worktree of plain agents with no lead looks normal: order kept, no task indent flag", () => {
    const list = [a(1), a(2)];
    expect(leadFirst(list).map((s) => s.id)).toEqual([1, 2]);
    expect(list.some((s) => isLead(s))).toBe(false);
  });
  it("finds the running lead of a worktree whatever its state and path spelling", () => {
    const at = (id: number, wt: string, over: Parameters<typeof a>[1] = {}) => {
      const s = a(id, over);
      return { ...s, meta: { ...s.meta, worktree: wt } } as unknown as import("./api").SessionInfo;
    };
    const wt = "C:\\Users\\me\\.axelstra-wt\\proj-ci-green-2";
    // idle lead plus a working task: the lead is there
    const list = [at(1, wt, { lead: true, state: "idle" }), at(2, wt, { state: "working" })];
    expect(leadOf(list, wt)?.id).toBe(1);
    // forward slashes, other case, trailing slash and the verbatim prefix all name the same folder
    for (const p of [
      "C:/Users/me/.axelstra-wt/proj-ci-green-2",
      "c:\\users\\ME\\.axelstra-wt\\proj-ci-green-2\\",
      "\\\\?\\C:\\Users\\me\\.axelstra-wt\\proj-ci-green-2",
    ])
      expect(leadOf(list, p)?.id).toBe(1);
    expect(leadOf([at(1, wt + "-other", { lead: true })], wt)).toBeUndefined();
    // an ended lead, a task, or a lead of another worktree does not count
    expect(leadOf([at(1, wt, { lead: true, running: false }), at(2, wt)], wt)).toBeUndefined();
    expect(leadOf([at(1, wt, { lead: true, kind: "shell" })], wt)).toBeUndefined();
  });
  it("lets the lead speak for the worktree dot; a waiting task still shows", () => {
    // tasks working, lead idle: the worktree is not "working"
    expect(worktreeSignals([a(1, { lead: true }), a(2, { state: "working" })], stateOf)).toEqual({
      live: 2,
      waiting: false,
      working: false,
      stalled: false,
      awaiting: false,
    });
    // lead working
    expect(worktreeSignals([a(1, { lead: true, state: "working" }), a(2)], stateOf).working).toBe(true);
    // a task asks something
    expect(worktreeSignals([a(1, { lead: true }), a(2, { state: "waiting" })], stateOf).waiting).toBe(true);
    // an awaiting agent shows as awaiting, and is never stalled
    const aw = { ...a(2), stalled: true, awaiting: { reason: "CI", since: 1, until: null } };
    expect(worktreeSignals([a(1, { lead: true }), aw], stateOf)).toMatchObject({ stalled: false, awaiting: true });
    // a stalled task (or lead) shows on the worktree
    expect(worktreeSignals([a(1, { lead: true }), { ...a(2), stalled: true }], stateOf).stalled).toBe(true);
    expect(
      worktreeSignals([a(1, { lead: true }), { ...a(2, { kind: "shell" }), stalled: true }], stateOf).stalled,
    ).toBe(false);
    expect(
      worktreeSignals([a(1, { lead: true }), { ...a(2, { running: false }), stalled: true }], stateOf).stalled,
    ).toBe(false);
    // no lead (older sessions): any agent counts, as before
    expect(worktreeSignals([a(1, { state: "working" })], stateOf)).toEqual({
      live: 1,
      waiting: false,
      working: true,
      stalled: false,
      awaiting: false,
    });
    // ended sessions do not count
    expect(worktreeSignals([a(1, { lead: true, running: false }), a(2)], stateOf).live).toBe(1);
  });
});

describe("autoSeatOf: what a selected worktree opens on", () => {
  type Row = { id: number; label: string; running: boolean; meta: SessionMeta };
  const r = (id: number, meta: SessionMeta, label = `Tarefa ${id}`, running = true): Row => ({
    id,
    label,
    running,
    meta,
  });
  const seat = (tabs: Row[]) => autoSeatOf(tabs)?.id ?? null;

  it("never a task or worker", () => {
    expect(
      seat([r(1, { kind: "agent" }), r(2, { kind: "agent", worker: true }), r(3, { kind: "agent", parent: 9 })]),
    ).toBeNull();
  });
  it("the lead, the project main or the row named main before a terminal", () => {
    expect(seat([r(1, { kind: "terminal" }), r(2, { kind: "agent" }), r(3, { kind: "agent", lead: true })])).toBe(3);
    expect(seat([r(1, { kind: "terminal" }), r(2, { kind: "agent", main: true }, "Agente")])).toBe(2);
    expect(seat([r(1, { kind: "agent" }), r(2, { kind: "agent" }, "main")])).toBe(2);
  });
  it("a worker named main is still a worker", () => {
    expect(seat([r(1, { kind: "agent", worker: true }, "main")])).toBeNull();
  });
  it("a running main or terminal first", () => {
    expect(
      seat([r(1, { kind: "agent", lead: true }, "main", false), r(2, { kind: "agent", lead: true }, "main")]),
    ).toBe(2);
    expect(seat([r(1, { kind: "terminal" }, "t", false), r(2, { kind: "terminal" }, "t")])).toBe(2);
  });
  it("a terminal when there is no main", () => {
    expect(seat([r(1, { kind: "agent" }), r(2, {}, "pwsh")])).toBe(2);
  });
});
