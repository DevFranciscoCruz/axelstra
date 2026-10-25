import { describe, expect, it } from "vitest";
import { isLead, lacksLead, leadFirst, worktreeSignals } from "./leadRole";
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
  it("offers to start a lead only in a worktree that has none running, never in the main checkout", () => {
    expect(lacksLead([], false)).toBe(true);
    expect(lacksLead([a(1)], false)).toBe(true);
    expect(lacksLead([a(1, { lead: true, running: false })], false)).toBe(true);
    expect(lacksLead([a(1, { lead: true })], false)).toBe(false);
    expect(lacksLead([], true)).toBe(false);
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
