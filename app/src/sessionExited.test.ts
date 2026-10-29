import { beforeEach, describe, expect, it, vi } from "vitest";
import type { SessionInfo } from "./api";

const invoke = vi.hoisted(() =>
  vi.fn(async (cmd: string, _args?: unknown) => (cmd === "notify_session" ? true : undefined)),
);
const now = vi.hoisted(() => ({ list: [] as unknown[] }));

vi.mock("@tauri-apps/api/core", () => ({ invoke }));
vi.mock("./Confirm", () => ({ ask: vi.fn(async () => true) }));
vi.mock("./api", () => ({
  listSessions: async () => now.list,
  listProjects: async () => [],
  worktreeStatus: async () => [],
  ghListBg: async () => [],
  gitChanges: async () => null,
  flowTasks: async () => [],
  worktreeOverview: async () => [],
  apiContext: { projects: () => [], terse: () => undefined },
  call: async () => ({}),
  callBg: async () => ({}),
}));

const WT = "C:/work/proj-a";
const sess = (over: Partial<SessionInfo>): SessionInfo => ({
  id: 1,
  running: true,
  idle_ms: 0,
  label: "t",
  meta: { kind: "terminal", worktree: WT },
  ...over,
});
const load = async () => {
  vi.resetModules();
  const store = await import("./store");
  const toasts = await import("./toasts");
  return { store, toasts };
};

beforeEach(() => invoke.mockClear());

describe("session.exited", () => {
  it("a terminal that failed keeps its tab, shows the code, and leaves on dismiss", async () => {
    const { store, toasts } = await load();
    now.list = [sess({ id: 4 })];
    await store.refreshSessions();
    now.list = []; // the daemon already dropped it
    store.handleSessionExited({ id: 4, code: 3 });
    await store.refreshSessions();
    const tab = store.sessionsOf(WT);
    expect(tab.map((s) => [s.id, s.running])).toEqual([[4, false]]);
    expect(toasts.toasts().some((t) => t.kind === "error" && t.text.includes("3"))).toBe(true);
    store.dismissExited(4);
    expect(store.sessionsOf(WT)).toEqual([]);
    await store.refreshSessions();
    expect(store.sessionsOf(WT)).toEqual([]);
  });

  it("a clean exit and an agent leave the list with no kept tab", async () => {
    const { store, toasts } = await load();
    now.list = [sess({ id: 1 }), sess({ id: 2, meta: { kind: "agent", worktree: WT } })];
    await store.refreshSessions();
    now.list = [];
    store.handleSessionExited({ id: 1, code: 0 });
    store.handleSessionExited({ id: 2, code: 1 });
    await store.refreshSessions();
    expect(store.sessionsOf(WT)).toEqual([]);
    // the agent's failure is still shown
    expect(toasts.toasts().filter((t) => t.kind === "error")).toHaveLength(1);
  });

  const agent = (id: number, over: Partial<SessionInfo> = {}) =>
    sess({ id, label: `Tarefa ${id}`, meta: { kind: "agent", worktree: WT }, ...over });

  it("an agent tab closes at once on session.exited, and a late stale list does not bring it back", async () => {
    const { store } = await load();
    now.list = [agent(1), agent(2)];
    await store.refreshSessions();
    const stale = [agent(1), agent(2)];
    store.handleSessionExited({ id: 2, code: 0 });
    expect(store.sessionsOf(WT).map((s) => s.id)).toEqual([1]);
    now.list = stale; // a poll that was already in flight
    await store.refreshSessions();
    expect(store.sessionsOf(WT).map((s) => s.id)).toEqual([1]);
    expect(store.exitCodeOf(2)).toBeUndefined();
  });

  it("focus moves to a neighbour only when the active tab closes", async () => {
    const { store } = await load();
    now.list = [agent(1), agent(2), agent(3)];
    await store.refreshSessions();
    store.activate(1, "user");
    store.handleSessionExited({ id: 3, code: 0 }); // not the active one
    expect(store.activeSession()).toBe(1);
    store.activate(2, "user");
    store.handleSessionExited({ id: 2, code: 0 }); // the active one: its neighbour
    expect(store.activeSession()).toBe(1);
    store.handleSessionExited({ id: 1, code: 0 });
    expect(store.activeSession()).toBeNull();
  });

  it("an agent that vanished from session.list (no event) closes too and the active tab moves on", async () => {
    const { store } = await load();
    now.list = [agent(1), agent(2), agent(3)];
    await store.refreshSessions();
    store.activate(2, "user");
    store.setSplitSession(3);
    now.list = [agent(1)];
    await store.refreshSessions();
    expect(store.sessionsOf(WT).map((s) => s.id)).toEqual([1]);
    expect(store.activeSession()).toBe(1);
    expect(store.splitSession()).toBeNull();
    // another tab vanishing does not steal focus
    now.list = [agent(1), agent(4)];
    await store.refreshSessions();
    store.activate(1, "user");
    now.list = [agent(1)];
    await store.refreshSessions();
    expect(store.activeSession()).toBe(1);
  });

  it("a failed terminal keeps its tab and code; an id the daemon lists again replaces it; sessionGone spares it", async () => {
    const { store } = await load();
    now.list = [sess({ id: 5 })];
    await store.refreshSessions();
    now.list = [];
    store.handleSessionExited({ id: 5, code: 2 });
    expect(store.exitCodeOf(5)).toBe(2);
    store.sessionGone(5);
    await store.refreshSessions();
    expect(store.sessionsOf(WT).map((s) => [s.id, s.running])).toEqual([[5, false]]);
    // after a daemon restart a new session gets the id
    now.list = [agent(5)];
    await store.refreshSessions();
    expect(store.sessionsOf(WT).map((s) => [s.id, s.running])).toEqual([[5, true]]);
    expect(store.exitCodeOf(5)).toBeUndefined();
  });

  it("sessionGone (no such session) closes an agent tab", async () => {
    const { store } = await load();
    now.list = [agent(7)];
    await store.refreshSessions();
    now.list = [];
    store.sessionGone(7);
    expect(store.sessionsOf(WT)).toEqual([]);
  });

  it("the lead button condition: an idle lead with a working task is a lead, whatever the path spelling", async () => {
    const { store } = await load();
    const lead = (path: string) =>
      sess({ id: 1, label: "main", idle_ms: 600000, meta: { kind: "agent", worktree: path, lead: true } });
    const task = sess({ id: 2, label: "Tarefa 1", idle_ms: 0, meta: { kind: "agent", worktree: WT } });
    const { canStartLead: lacksLead } = await import("./leadRole");
    for (const p of [WT, "C:\\work\\proj-a", "c:/WORK/proj-a/", "\\\\?\\C:\\work\\proj-a"]) {
      now.list = [lead(p), task];
      await store.refreshSessions();
      expect(
        lacksLead(
          store.sessionsOf(WT).filter((s) => s.meta?.kind === "agent"),
          false,
        ),
      ).toBe(false);
    }
    now.list = [task];
    await store.refreshSessions();
    expect(
      lacksLead(
        store.sessionsOf(WT).filter((s) => s.meta?.kind === "agent"),
        false,
      ),
    ).toBe(true);
  });

  it("an id the app never showed is ignored", async () => {
    const { store, toasts } = await load();
    now.list = [];
    store.handleSessionExited({ id: 9, code: 2 });
    expect(toasts.toasts()).toEqual([]);
  });
});
