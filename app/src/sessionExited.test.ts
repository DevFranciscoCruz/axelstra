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

  it("an id the app never showed is ignored", async () => {
    const { store, toasts } = await load();
    now.list = [];
    store.handleSessionExited({ id: 9, code: 2 });
    expect(toasts.toasts()).toEqual([]);
  });
});
