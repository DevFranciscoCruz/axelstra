import { describe, expect, it } from "vitest";
import type { ShipStatus } from "./api";
import { MAX_ENTRIES, NotifEntry, addEntry, isDuplicate, parseStored, shipEvents, trim, unreadOf } from "./notifications";

const entry = (i: number, over: Partial<NotifEntry> = {}): NotifEntry => ({ id: `e${i}`, at: i, kind: "done", worktree: "/w", text: `t${i}`, read: false, ...over });

describe("trim", () => {
  it("keeps the newest 100", () => {
    const l = Array.from({ length: 130 }, (_, i) => entry(130 - i));
    const t = trim(l);
    expect(t).toHaveLength(MAX_ENTRIES);
    expect(t[0].id).toBe("e130");
  });
});

describe("addEntry", () => {
  it("puts the new one first and keeps the cap", () => {
    let l: NotifEntry[] = [];
    for (let i = 0; i < 120; i++) l = addEntry(l, { kind: "error", worktree: "/w", text: `n${i}` }, 1000 + i, `id${i}`);
    expect(l).toHaveLength(MAX_ENTRIES);
    expect(l[0].text).toBe("n119");
    expect(l[0].read).toBe(false);
  });
  it("drops a repeat inside the window but not after it", () => {
    const e = { kind: "waiting" as const, worktree: "/w", text: "x" };
    const l1 = addEntry([], e, 1000, "a");
    expect(addEntry(l1, e, 2000, "b")).toBe(l1);
    expect(addEntry(l1, e, 1000 + 3 * 60_000, "c")).toHaveLength(2);
    expect(isDuplicate(l1, { ...e, worktree: "/other" }, 2000)).toBe(false);
    expect(isDuplicate(l1, { ...e, kind: "done" }, 2000)).toBe(false);
  });
});

describe("parseStored", () => {
  it("drops the old idle noise logged as errors", () => {
    const e = (id: string, kind: string, text: string) => ({ id, at: 1, kind, worktree: "w", text, read: false });
    const raw = JSON.stringify([e("a", "error", "Tarefa 8 ficou parado na worktree x"), e("b", "error", "Falhou o merge"), e("c", "done", "ok")]);
    expect(parseStored(raw).map((x) => x.id).sort()).toEqual(["b", "c"]);
  });

  it("survives garbage", () => {
    expect(parseStored(null)).toEqual([]);
    expect(parseStored("nope")).toEqual([]);
    expect(parseStored("{}")).toEqual([]);
  });
  it("drops bad entries and sorts newest first", () => {
    const raw = JSON.stringify([entry(1), { id: 3 }, entry(5), { ...entry(2), kind: "weird" }, null]);
    expect(parseStored(raw).map((e) => e.id)).toEqual(["e5", "e1"]);
  });
  it("caps what it loads", () => {
    const raw = JSON.stringify(Array.from({ length: 150 }, (_, i) => entry(i)));
    expect(parseStored(raw)).toHaveLength(MAX_ENTRIES);
  });
});

describe("unreadOf", () => {
  it("counts the unread", () => {
    expect(unreadOf([entry(1), entry(2, { read: true }), entry(3)])).toBe(2);
  });
});

const st = (over: Partial<ShipStatus> = {}): ShipStatus => ({
  problem: "", detail: "", number: 7, title: "Fix", url: "u", pr_state: "open", draft: false, ci: "pending", checks: [], review: "", mergeable: "yes", merge_state: "clean", can_merge: false, ...over,
});

describe("shipEvents", () => {
  it("first read and a new PR only set the baseline", () => {
    expect(shipEvents(null, st({ ci: "fail" }), "/w", "feat")).toEqual([]);
    expect(shipEvents(st({ number: 6 }), st({ ci: "fail" }), "/w", "feat")).toEqual([]);
  });
  it("reports CI turning failing or passing", () => {
    expect(shipEvents(st(), st({ ci: "fail" }), "/w", "feat").map((e) => e.kind)).toEqual(["ci-fail"]);
    expect(shipEvents(st(), st({ ci: "pass" }), "/w", "feat").map((e) => e.kind)).toEqual(["ci-pass"]);
    expect(shipEvents(st({ ci: "fail" }), st({ ci: "pass" }), "/w", "feat").map((e) => e.kind)).toEqual(["ci-pass"]);
  });
  it("says nothing when CI first appears as passing, or does not change", () => {
    expect(shipEvents(st({ ci: "none" }), st({ ci: "pass" }), "/w", "feat")).toEqual([]);
    expect(shipEvents(st(), st(), "/w", "feat")).toEqual([]);
  });
  it("reports a merge, once", () => {
    const e = shipEvents(st({ ci: "pass" }), st({ ci: "pass", pr_state: "merged" }), "/w", "feat");
    expect(e.map((x) => x.kind)).toEqual(["merged"]);
    expect(e[0].worktree).toBe("/w");
    expect(shipEvents(st({ pr_state: "merged" }), st({ pr_state: "merged" }), "/w", "feat")).toEqual([]);
  });
  it("ignores problems", () => {
    expect(shipEvents(st(), st({ problem: "auth" }), "/w", "feat")).toEqual([]);
    expect(shipEvents(st({ problem: "nopr" }), st({ ci: "fail" }), "/w", "feat")).toEqual([]);
  });
});
