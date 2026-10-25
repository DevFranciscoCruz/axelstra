import { describe, expect, it } from "vitest";
import { COL_IDS, applyPlan, cmpCards, colAt, columnCards, edgeDelta, indexAt, isBypass, keyStep, planMove, planStart, posOf, restorePayload } from "./kanbanLogic";
import type { KCard } from "./api";
import { needsTypedName, removeRisks } from "./removeRisk";
import { typedMatches } from "./Confirm";

const card = (id: string, mode: string, bypass = false, acceptance: string | null = null) => ({
  id,
  agent: "claude",
  permission_mode: mode,
  bypass,
  acceptance,
});

describe("planStart", () => {
  it("shows each card's mode, flags bypass and sends the pin", () => {
    const p = planStart(
      2,
      { pin: "abc", cards: [card("a", "default"), card("b", "bypassPermissions", true, "make test")] },
      [],
    );
    expect(p.message).toContain("⚠ b: claude (bypassPermissions)");
    expect(p.message).toContain("a: claude (default)");
    expect(p.bypass).toEqual(["b"]);
    expect(p.pin).toBe("abc");
    expect(p.trustAcceptance).toBe(true);
    expect(p.warn).toBe(true);
  });
  it("detects bypass by mode name even when the flag is missing", () => {
    expect(isBypass("bypass-permissions")).toBe(true);
    expect(isBypass("acceptEdits")).toBe(false);
    expect(planStart(1, { pin: "p", cards: [card("a", "bypassPermissions")] }, []).bypass).toEqual(["a"]);
  });
  it("plain cards: no warning, no pin without trust info", () => {
    expect(planStart(1, { pin: "p", cards: [card("a", "default")] }, []).warn).toBe(false);
    const legacy = planStart(1, null, ["a: x"]);
    expect(legacy.pin).toBeUndefined();
    expect(legacy.trustAcceptance).toBe(true);
  });
});

describe("restorePayload", () => {
  it("keeps the id and every field", () => {
    const p = restorePayload({
      meta: {
        id: "k1",
        title: "T",
        status: "todo",
        agent: "claude",
        depends_on: ["k0"],
        priority: 2,
        acceptance: "ok",
      },
      body: "b",
    });
    expect(p).toEqual({
      id: "k1",
      title: "T",
      body: "b",
      status: "todo",
      agent: "claude",
      priority: 2,
      depends_on: ["k0"],
      acceptance: "ok",
    });
  });
});

describe("remove worktree confirm", () => {
  it("types the name only when work would be lost", () => {
    expect(needsTypedName({ dirty: 2, ahead: 0, upstream: true })).toBe(true);
    expect(needsTypedName({ dirty: 0, ahead: 1, upstream: false })).toBe(true);
    expect(needsTypedName({ dirty: 0, ahead: 0, upstream: true })).toBe(false);
    expect(needsTypedName(undefined)).toBe(false);
  });
  it("describes the risks", () => {
    expect(removeRisks({ dirty: 1, ahead: 2, upstream: false }, 1)).toHaveLength(3);
  });
  it("typedMatches ignores case and spaces but never an empty target", () => {
    expect(typedMatches("Foo", " foo ")).toBe(true);
    expect(typedMatches("Foo", "fo")).toBe(false);
    expect(typedMatches("", "")).toBe(false);
  });
});

const kc = (id: string, status: string, priority = 0): KCard => ({
  meta: { id, title: id, status, agent: "claude", depends_on: [], priority, acceptance: null },
  body: "",
});
const ids = (cards: KCard[], col: string) => columnCards(cards, col).map((c) => c.meta.id);
/** the board after the plan is written: what the daemon would list */
const after = (cards: KCard[], id: string, plan: ReturnType<typeof planMove>) => (plan ? applyPlan(cards, id, plan) : cards);

describe("order", () => {
  it("sorts like the daemon: priority descending, then id", () => {
    const cards = [kc("b", "todo", 0), kc("a", "todo", 0), kc("z", "todo", 5)];
    expect(ids(cards, "todo")).toEqual(["z", "a", "b"]);
    expect(cmpCards({ id: "a", priority: 1 }, { id: "a", priority: 1 })).toBe(0);
  });
  it("shows an unknown status under the first column", () => {
    expect(ids([kc("x", "weird"), kc("y", "todo")], "backlog")).toEqual(["x"]);
    expect(posOf([kc("x", "weird")], "x")).toEqual({ col: "backlog", index: 0 });
  });
});

describe("pointer targets", () => {
  it("counts the cards whose middle is above the pointer", () => {
    expect(indexAt([10, 30, 50], 5)).toBe(0);
    expect(indexAt([10, 30, 50], 31)).toBe(2);
    expect(indexAt([10, 30, 50], 99)).toBe(3);
    expect(indexAt([], 5)).toBe(0);
  });
  it("finds the column, null outside or in a gap", () => {
    const r: [number, number][] = [[0, 100], [108, 200]];
    expect(colAt(r, 50)).toBe(0);
    expect(colAt(r, 150)).toBe(1);
    expect(colAt(r, 104)).toBeNull();
    expect(colAt(r, 500)).toBeNull();
  });
  it("scrolls only near an edge, faster the closer", () => {
    expect(edgeDelta(500, 0, 1000)).toBe(0);
    expect(edgeDelta(10, 0, 1000)).toBeLessThan(0);
    expect(edgeDelta(990, 0, 1000)).toBeGreaterThan(0);
    expect(Math.abs(edgeDelta(2, 0, 1000))).toBeGreaterThan(Math.abs(edgeDelta(40, 0, 1000)));
    expect(edgeDelta(5, 0, 60)).toBe(0);
  });
});

describe("keyStep", () => {
  const lens = [2, 0, 3, 1, 0];
  it("moves inside a column and clamps", () => {
    expect(keyStep({ col: "backlog", index: 0 }, "ArrowDown", COL_IDS, lens)).toEqual({ col: "backlog", index: 1 });
    expect(keyStep({ col: "backlog", index: 2 }, "ArrowDown", COL_IDS, lens)).toEqual({ col: "backlog", index: 2 });
    expect(keyStep({ col: "backlog", index: 0 }, "ArrowUp", COL_IDS, lens)).toEqual({ col: "backlog", index: 0 });
    expect(keyStep({ col: "in-progress", index: 1 }, "Home", COL_IDS, lens)).toEqual({ col: "in-progress", index: 0 });
    expect(keyStep({ col: "in-progress", index: 1 }, "End", COL_IDS, lens)).toEqual({ col: "in-progress", index: 3 });
  });
  it("moves between columns keeping the index as far as it fits", () => {
    expect(keyStep({ col: "backlog", index: 2 }, "ArrowRight", COL_IDS, lens)).toEqual({ col: "todo", index: 0 });
    expect(keyStep({ col: "todo", index: 0 }, "ArrowRight", COL_IDS, lens)).toEqual({ col: "in-progress", index: 0 });
    expect(keyStep({ col: "in-progress", index: 3 }, "ArrowLeft", COL_IDS, lens)).toEqual({ col: "todo", index: 0 });
    expect(keyStep({ col: "backlog", index: 1 }, "ArrowLeft", COL_IDS, lens)).toEqual({ col: "backlog", index: 1 });
    expect(keyStep({ col: "done", index: 0 }, "ArrowRight", COL_IDS, lens)).toEqual({ col: "done", index: 0 });
  });
  it("ignores other keys", () => {
    expect(keyStep({ col: "todo", index: 0 }, "a", COL_IDS, lens)).toEqual({ col: "todo", index: 0 });
  });
});

describe("planMove", () => {
  it("is a no-op when the card stays where it is", () => {
    const cards = [kc("a", "todo"), kc("b", "todo")];
    expect(planMove(cards, "a", { col: "todo", index: 0 })).toBeNull();
    expect(planMove(cards, "b", { col: "todo", index: 1 })).toBeNull();
    expect(planMove(cards, "nope", { col: "todo", index: 0 })).toBeNull();
  });
  it("changing column alone writes the status and no priority when the card still fits", () => {
    const cards = [kc("a", "todo"), kc("b", "done")];
    const p = planMove(cards, "a", { col: "done", index: 0 })!;
    expect(p).toEqual({ status: "done", priorities: [] });
    expect(ids(after(cards, "a", p), "done")).toEqual(["a", "b"]);
  });
  it("reorders inside a column with one priority write", () => {
    const cards = [kc("a", "todo", 2), kc("b", "todo", 2), kc("c", "todo", 0)];
    const p = planMove(cards, "c", { col: "todo", index: 0 })!;
    expect(p.status).toBeNull();
    expect(p.priorities).toHaveLength(1);
    expect(ids(after(cards, "c", p), "todo")).toEqual(["c", "a", "b"]);
  });
  it("puts a card between two cards with the same priority by renumbering when nothing fits", () => {
    const cards = [kc("a", "todo", 1), kc("b", "todo", 1), kc("z", "todo", 1)];
    const p = planMove(cards, "z", { col: "todo", index: 1 })!;
    expect(ids(after(cards, "z", p), "todo")).toEqual(["a", "z", "b"]);
    const prios = columnCards(after(cards, "z", p), "todo").map((c) => c.meta.priority);
    expect(new Set(prios).size).toBe(3);
  });
  it("moves to the end and to the top of other columns", () => {
    const cards = [kc("a", "todo", 0), kc("b", "done", 3), kc("c", "done", 1)];
    const end = planMove(cards, "a", { col: "done", index: 2 })!;
    expect(ids(after(cards, "a", end), "done")).toEqual(["b", "c", "a"]);
    const top = planMove(cards, "a", { col: "done", index: 0 })!;
    expect(ids(after(cards, "a", top), "done")).toEqual(["a", "b", "c"]);
  });
  it("is stable: every slot of every move yields exactly that order, and replaying it is a no-op", () => {
    const base = [kc("a", "todo", 1), kc("b", "todo", 1), kc("c", "todo", 1), kc("d", "todo", 0), kc("e", "done", 0)];
    for (const id of ["a", "b", "c", "d", "e"]) {
      for (const col of ["todo", "done"]) {
        const rest = columnCards(base, col).filter((c) => c.meta.id !== id).map((c) => c.meta.id);
        for (let i = 0; i <= rest.length; i++) {
          const p = planMove(base, id, { col, index: i });
          const next = after(base, id, p);
          expect(ids(next, col)).toEqual([...rest.slice(0, i), id, ...rest.slice(i)]);
          expect(planMove(next, id, { col, index: i })).toBeNull();
        }
      }
    }
  });
  it("only rewrites cards whose priority changes", () => {
    const cards = [kc("a", "todo", 5), kc("b", "todo", 5), kc("c", "todo", 5)];
    const p = planMove(cards, "c", { col: "todo", index: 0 })!;
    expect(p.priorities.every((x) => cards.find((c) => c.meta.id === x.id)!.meta.priority !== x.priority)).toBe(true);
  });
  it("moves an unknown-status card into backlog by status", () => {
    const cards = [kc("x", "weird")];
    expect(planMove(cards, "x", { col: "backlog", index: 0 })).toEqual({ status: "backlog", priorities: [] });
  });
});
