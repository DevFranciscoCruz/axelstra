import { beforeEach, describe, expect, it, vi } from "vitest";
import { createEffect, createRoot } from "solid-js";
import type { SessionInfo } from "./api";

const invoke = vi.hoisted(() => vi.fn(async (cmd: string) => (cmd === "notify_session" ? true : undefined)));
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
const agent = (id: number, over: Partial<SessionInfo> = {}): SessionInfo => ({
  id,
  running: true,
  idle_ms: 0,
  label: `Tarefa ${id}`,
  meta: { kind: "agent", worktree: WT },
  ...over,
});
const load = async () => {
  vi.resetModules();
  const store = await import("./store");
  const tabs = () => store.sortedTabs(store.sessionsOf(WT));
  return { store, tabs };
};
const poll = async (store: Awaited<ReturnType<typeof load>>["store"], list: SessionInfo[]) => {
  now.list = list;
  await store.refreshSessions();
};

beforeEach(() => {
  invoke.mockClear();
  // selectWorktree persists the choice and a fresh module load restores it: never let one test seed the next
  try {
    localStorage.clear(); // some runtimes expose no usable localStorage: then nothing leaks either
  } catch {
    /* no storage */
  }
  now.list = [];
});

describe("only the user changes the active session", () => {
  it("restore: seats the worktree's agent once the first list arrived, not before", async () => {
    const { store, tabs } = await load();
    store.settleActive(tabs());
    expect(store.activeSession()).toBeNull();
    await poll(store, [agent(1, { meta: { kind: "terminal", worktree: WT } }), agent(2)]);
    store.settleActive(tabs());
    expect(store.activeSession()).toBe(2);
  });

  it("a session spawned by an agent, worker or dispatch never takes the focus", async () => {
    const { store, tabs } = await load();
    await poll(store, [agent(1)]);
    store.settleActive(tabs());
    expect(store.activeSession()).toBe(1);
    await poll(store, [agent(1), agent(2, { meta: { kind: "agent", worktree: WT, worker: true } as never }), agent(3)]);
    store.settleActive(tabs());
    expect(store.activeSession()).toBe(1);
  });

  it("an empty seat is not filled by a later spawn (all tabs closed, then a worker appears)", async () => {
    const { store, tabs } = await load();
    await poll(store, [agent(1)]);
    store.settleActive(tabs());
    store.handleSessionExited({ id: 1, code: 0 });
    expect(store.activeSession()).toBeNull();
    await poll(store, [agent(5)]);
    store.settleActive(tabs());
    expect(store.activeSession()).toBeNull();
  });

  it("an empty worktree the user selected stays empty when a worker spawns into it", async () => {
    const { store, tabs } = await load();
    await poll(store, []);
    store.settleActive(tabs()); // first list: nothing to seat
    store.selectWorktree(WT);
    store.settleActive(tabs()); // selection settles with no tabs
    await poll(store, [agent(8)]);
    store.settleActive(tabs());
    expect(store.activeSession()).toBeNull();
  });

  it("selecting a worktree seats its agent; the user's own pick wins", async () => {
    const { store, tabs } = await load();
    await poll(store, [agent(1), agent(2)]);
    store.settleActive(tabs());
    store.activate(1, "user");
    store.selectWorktree(WT);
    store.settleActive(tabs());
    expect(store.activeSession()).toBe(1);
    store.activate(2, "user");
    store.settleActive(tabs());
    expect(store.activeSession()).toBe(2);
  });

  it("auto never replaces a live active session", async () => {
    const { store } = await load();
    await poll(store, [agent(1), agent(2)]);
    store.activate(1, "user");
    expect(store.activate(2, "auto")).toBe(false);
    expect(store.activeSession()).toBe(1);
    store.armAutoPick();
    expect(store.activate(2, "auto")).toBe(false);
    expect(store.activeSession()).toBe(1);
  });

  it("session.exited of a non-active session keeps the active one; of the active one moves once to the neighbour", async () => {
    const { store } = await load();
    await poll(store, [agent(1), agent(2), agent(3)]);
    store.activate(2, "user");
    store.handleSessionExited({ id: 3, code: 0 });
    expect(store.activeSession()).toBe(2);
    store.handleSessionExited({ id: 2, code: 0 });
    expect(store.activeSession()).toBe(1);
  });

  it("a reordered or refreshed list (new sessions, changed state) keeps the active one", async () => {
    const { store, tabs } = await load();
    await poll(store, [agent(1), agent(2), agent(3)]);
    store.activate(3, "user");
    await poll(store, [agent(3, { idle_ms: 900000 }), agent(4), agent(2), agent(1)]);
    store.settleActive(tabs());
    expect(store.activeSession()).toBe(3);
  });

  it("hibernate and wake keep the row, so the active session stays; a hibernated row is not a switch", async () => {
    const { store, tabs } = await load();
    await poll(store, [agent(1), agent(2)]);
    store.activate(1, "user");
    const sleeping = { hibernated: { since: 1, saved_mb: 10, waking: false, queued: false } } as Partial<SessionInfo>;
    await poll(store, [agent(1), agent(2, sleeping)]);
    store.settleActive(tabs());
    expect(store.activeSession()).toBe(1);
    await poll(store, [agent(1, sleeping), agent(2)]);
    store.settleActive(tabs());
    expect(store.activeSession()).toBe(1);
    await poll(store, [agent(1), agent(2)]);
    expect(store.activeSession()).toBe(1);
  });

  it("an active id the list lost without a neighbour empties the seat instead of jumping", async () => {
    const { store, tabs } = await load();
    await poll(store, [agent(1)]);
    store.activate(1, "user");
    await poll(store, [agent(9, { meta: { kind: "agent", worktree: "C:/work/other" } })]);
    store.settleActive(tabs());
    expect(store.activeSession()).toBeNull();
  });

  it("after a daemon restart the app may seat a session again", async () => {
    const { store, tabs } = await load();
    await poll(store, [agent(1)]);
    store.settleActive(tabs());
    await poll(store, []);
    store.settleActive(tabs());
    expect(store.activeSession()).toBeNull();
    store.armAutoPick();
    await poll(store, [agent(1)]);
    store.settleActive(tabs());
    expect(store.activeSession()).toBe(1);
  });

  it("the split pane swaps when the user activates the split session", async () => {
    const { store } = await load();
    await poll(store, [agent(1), agent(2)]);
    store.activate(1, "user");
    store.setSplitSession(2);
    store.activate(2, "user");
    expect(store.activeSession()).toBe(2);
    expect(store.splitSession()).toBe(1);
  });

  it("selecting a worktree after an empty settle seats a tab through the live effect (e2e: terminal must mount)", async () => {
    const { store, tabs } = await load();
    await poll(store, [agent(1, { meta: { kind: "terminal", worktree: WT } })]);
    const dispose = createRoot((d) => {
      // same wiring as App.tsx: the effect reads the selected worktree's tabs
      createEffect(() => store.settleActive(store.sortedTabs(store.sessionsOf(store.selectedWorktree()))));
      return d;
    });
    await Promise.resolve();
    expect(store.activeSession()).toBeNull(); // no worktree yet: the empty settle used the armed pick up
    store.selectWorktree(WT);
    expect(tabs().length).toBe(1);
    expect(store.activeSession()).toBe(1);
    dispose();
  });
});
