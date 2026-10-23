import { describe, expect, it } from "vitest";
import { groupRunning, isFinished, layout, wtKey, type GroupWt } from "./wtGroups";

const wt = (name: string, extra: Partial<GroupWt> = {}): GroupWt => ({ path: `C:/x/${name}`, branch: `ax/${name}`, is_main: false, ...extra });
const none = new Set<string>();
const set = (...names: string[]) => new Set(names.map((n) => wtKey(`C:/x/${n}`)));
const names = (rows: { wt: GroupWt; depth: number }[]) => rows.map((r) => `${"-".repeat(r.depth)}${r.wt.path.slice(5)}`);

describe("isFinished", () => {
  const merged = set("a");
  it("needs a merged branch and no running session", () => {
    expect(isFinished(wt("a"), merged, 0)).toBe(true);
    expect(isFinished(wt("a"), merged, 1)).toBe(false);
    expect(isFinished(wt("b"), merged, 0)).toBe(false);
  });
  it("never counts the main worktree", () => {
    expect(isFinished(wt("a", { is_main: true }), merged, 0)).toBe(false);
  });
  it("matches paths loosely", () => {
    expect(isFinished({ ...wt("a"), path: "c:\\X\\A\\" }, merged, 0)).toBe(true);
  });
});

describe("layout", () => {
  it("puts main first, then pinned, then the rest in order", () => {
    const all = [wt("a"), wt("m", { is_main: true }), wt("b"), wt("c")];
    const { rows } = layout(all, { finished: none, pinned: (p) => p.endsWith("/c") });
    expect(names(rows)).toEqual(["m", "c", "a", "b"]);
  });
  it("nests children under their parent", () => {
    const all = [wt("a"), wt("b", { parent: "C:/x/a" }), wt("c", { parent: "C:/x/b" }), wt("d")];
    const { rows } = layout(all, { finished: none });
    expect(names(rows)).toEqual(["a", "-b", "--c", "d"]);
    expect(rows[0].kids).toBe(1);
    expect(rows[0].members).toEqual([wtKey("C:/x/a"), wtKey("C:/x/b"), wtKey("C:/x/c")]);
  });
  it("hides the children of a collapsed parent but keeps the group's members", () => {
    const all = [wt("a"), wt("b", { parent: "C:/x/a" })];
    const { rows } = layout(all, { finished: none, collapsed: set("a") });
    expect(names(rows)).toEqual(["a"]);
    expect(rows[0].members.length).toBe(2);
  });
  it("moves finished worktrees to their own list, children included", () => {
    const all = [wt("a"), wt("b", { parent: "C:/x/a" }), wt("c")];
    const r = layout(all, { finished: set("b", "c") });
    expect(names(r.rows)).toEqual(["a"]);
    expect(r.rows[0].kids).toBe(0);
    expect(r.finished.map((w) => w.path)).toEqual(["C:/x/b", "C:/x/c"]);
  });
  it("shows an unfinished child at the top level when its parent is finished", () => {
    const all = [wt("a"), wt("b", { parent: "C:/x/a" })];
    const r = layout(all, { finished: set("a") });
    expect(names(r.rows)).toEqual(["b"]);
    expect(r.finished.length).toBe(1);
  });
  it("shows a child whose parent is unknown or hidden at the top level", () => {
    const all = [wt("a", { parent: "C:/x/gone" }), wt("b", { parent: "C:/x/h" }), wt("h")];
    const { rows } = layout(all, { finished: none, hidden: set("h") });
    expect(names(rows)).toEqual(["a", "b"]);
  });
  it("survives a loop of parents without losing worktrees", () => {
    const all = [wt("a", { parent: "C:/x/b" }), wt("b", { parent: "C:/x/a" }), wt("s", { parent: "C:/x/s" })];
    const { rows } = layout(all, { finished: none });
    expect(rows.map((r) => r.wt.path).sort()).toEqual(["C:/x/a", "C:/x/b", "C:/x/s"]);
  });
  it("never lists the main worktree as finished", () => {
    const all = [wt("m", { is_main: true })];
    const r = layout(all, { finished: set("m") });
    expect(r.finished).toEqual([]);
    expect(names(r.rows)).toEqual(["m"]);
  });
});

describe("groupRunning", () => {
  it("adds the running agents of every member", () => {
    const counts: Record<string, number> = { [wtKey("C:/x/a")]: 1, [wtKey("C:/x/b")]: 2 };
    expect(groupRunning([wtKey("C:/x/a"), wtKey("C:/x/b"), wtKey("C:/x/c")], (k) => counts[k] ?? 0)).toBe(3);
  });
});
