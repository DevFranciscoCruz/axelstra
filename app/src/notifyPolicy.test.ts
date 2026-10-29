import { describe, expect, it } from "vitest";
import { AGGREGATE_MS, EPISODE_MS, NewEntry, NotifEntry, addEntry, unreadOf } from "./notifications";
import { CATEGORIES, DEFAULT_PREFS, categoryOf, deliveryOf, parsePrefs } from "./notifyPrefs";
import { dotOfSession } from "./agentState";
import type { SessionInfo } from "./api";

const NOW = 1_000_000_000_000;
const e = (o: Partial<NewEntry> = {}): NewEntry => ({
  kind: "done",
  worktree: "/wt/a",
  text: "a terminou",
  session: 1,
  ...o,
});
const add = (l: NotifEntry[], n: NewEntry, at = NOW, read = false) =>
  addEntry(l, { ...n, at }, at, `id${l.length}-${at}`, read);

describe("categories and defaults", () => {
  it("is silent by default except 'need' and 'error'", () => {
    expect(DEFAULT_PREFS).toEqual({ need: true, done: false, info: false, error: true, dnd: false });
  });
  it("maps every kind to a category", () => {
    expect(categoryOf("waiting")).toBe("need");
    expect(categoryOf("attention")).toBe("need");
    expect(categoryOf("done")).toBe("done");
    expect(categoryOf("error")).toBe("error");
    expect(categoryOf("ci-fail")).toBe("error");
    expect(categoryOf("ci-pass")).toBe("info");
    expect(categoryOf("merged")).toBe("info");
    expect(categoryOf("budget")).toBe("info");
  });
  it("a switched-off category is recorded read and never interrupts", () => {
    expect(deliveryOf("done", DEFAULT_PREFS)).toEqual({ unread: false, interrupt: false });
    expect(deliveryOf("info", DEFAULT_PREFS)).toEqual({ unread: false, interrupt: false });
    expect(deliveryOf("need", DEFAULT_PREFS)).toEqual({ unread: true, interrupt: true });
    expect(deliveryOf("error", DEFAULT_PREFS)).toEqual({ unread: true, interrupt: true });
  });
  it("do not disturb records unread but never interrupts, in every category", () => {
    const p = { ...DEFAULT_PREFS, done: true, info: true, dnd: true };
    for (const c of CATEGORIES) expect(deliveryOf(c, p)).toEqual({ unread: true, interrupt: false });
  });
  it("parses saved prefs tolerating garbage", () => {
    expect(parsePrefs(null)).toEqual(DEFAULT_PREFS);
    expect(parsePrefs("not json")).toEqual(DEFAULT_PREFS);
    expect(parsePrefs("[1]")).toEqual(DEFAULT_PREFS);
    expect(parsePrefs('{"done":true,"need":"yes","dnd":true}')).toEqual({ ...DEFAULT_PREFS, done: true, dnd: true });
  });
});

describe("aggregation and dedupe", () => {
  it("folds several finished sessions into one entry", () => {
    let l: NotifEntry[] = [];
    l = add(l, e({ session: 1, worktree: "/wt/a" }), NOW);
    l = add(l, e({ session: 2, worktree: "/wt/b", text: "b terminou" }), NOW + 1000);
    l = add(l, e({ session: 3, worktree: "/wt/c", text: "c terminou" }), NOW + 2000);
    expect(l).toHaveLength(1);
    expect(l[0].n).toBe(3);
    expect(l[0].text).toContain("3");
    expect(l[0].worktree).toBe("");
    expect(l[0].session).toBeUndefined();
  });
  it("keeps the worktree when the folded events share it", () => {
    let l: NotifEntry[] = [];
    l = add(l, e({ session: 1, text: "x" }), NOW);
    l = add(l, e({ session: 2, text: "y" }), NOW + 1000);
    expect(l[0].worktree).toBe("/wt/a");
    expect(l[0].n).toBe(2);
  });
  it("does not fold after the window, nor across kinds, nor read with unread", () => {
    let l: NotifEntry[] = [];
    l = add(l, e(), NOW);
    l = add(l, e({ text: "later" }), NOW + AGGREGATE_MS + 1);
    expect(l).toHaveLength(2);
    l = add(l, e({ kind: "merged", text: "m" }), NOW + AGGREGATE_MS + 2);
    expect(l).toHaveLength(3);
    l = add(l, e({ text: "silent" }), NOW + AGGREGATE_MS + 3, true);
    expect(l).toHaveLength(4);
    expect(unreadOf(l)).toBe(3);
  });
  it("never aggregates things that need a person", () => {
    let l: NotifEntry[] = [];
    l = add(l, e({ kind: "waiting", text: "q1" }), NOW);
    l = add(l, e({ kind: "waiting", text: "q2", session: 2 }), NOW + 1000);
    expect(l).toHaveLength(2);
  });
  it("drops an identical entry twice in a row", () => {
    let l: NotifEntry[] = [];
    l = add(l, e({ kind: "error", text: "boom" }), NOW);
    l = add(l, e({ kind: "error", text: "boom" }), NOW + 1000);
    expect(l).toHaveLength(1);
  });
  it("one episode key is one notification, even with different text", () => {
    let l: NotifEntry[] = [];
    l = add(l, e({ kind: "waiting", text: "parado 5 min", key: "stall:7" }), NOW);
    l = add(l, e({ kind: "waiting", text: "parado 6 min", key: "stall:7" }), NOW + 10 * 60_000);
    expect(l).toHaveLength(1);
    l = add(l, e({ kind: "waiting", text: "parado 40 min", key: "stall:7" }), NOW + EPISODE_MS + 1);
    expect(l).toHaveLength(2);
  });
});

describe("stalled vs stall_parent", () => {
  const base = { id: 1, running: true, label: "x", meta: { kind: "agent" } } as SessionInfo;
  it("only `stalled` (needs a person) shows the stalled state", () => {
    expect(dotOfSession({ ...base, stalled: true })).toBe("stalled");
    expect(dotOfSession({ ...base, stalled: false, stall_parent: true })).not.toBe("stalled");
  });
});
