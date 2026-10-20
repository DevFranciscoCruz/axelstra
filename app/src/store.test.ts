import { beforeEach, describe, expect, it, vi } from "vitest";
import type { SessionInfo } from "./api";

const invoke = vi.hoisted(() => vi.fn(async (cmd: string, _args?: unknown) => (cmd === "notify_session" ? true : undefined)));
const sessionsNow = vi.hoisted(() => ({ list: [] as unknown[] }));

vi.mock("@tauri-apps/api/core", () => ({ invoke }));
vi.mock("./Confirm", () => ({ ask: vi.fn(async () => true) }));
vi.mock("./api", () => ({
  listSessions: async () => sessionsNow.list,
  listProjects: async () => [],
  worktreeStatus: async () => [],
  ghListBg: async () => [],
  gitChanges: async () => null,
}));

const WT = "C:/work/proj-a";
const OTHER = "C:/work/proj-b";
const agent = (over: Partial<SessionInfo> = {}): SessionInfo => ({
  id: 1,
  running: true,
  idle_ms: 0,
  waiting: false,
  label: "claude",
  meta: { kind: "agent", agent: "claude", worktree: WT },
  ...over,
});
const load = async () => {
  vi.resetModules();
  return await import("./store");
};
const tick = async (store: Awaited<ReturnType<typeof load>>, s: SessionInfo) => {
  sessionsNow.list = [s];
  await store.refreshSessions();
};
// notify() imports the Tauri module first, so the call lands a moment after refreshSessions returns
const settle = () => new Promise((r) => setTimeout(r, 30));
const notified = async () => {
  await settle();
  return invoke.mock.calls.filter((c) => c[0] === "notify_session").length;
};

// newer Node ships its own (unusable) localStorage that shadows jsdom's: use a plain in-memory one
const memory = () => {
  const m = new Map<string, string>();
  return { getItem: (k: string) => m.get(k) ?? null, setItem: (k: string, v: string) => void m.set(k, v), removeItem: (k: string) => void m.delete(k), clear: () => m.clear() };
};

beforeEach(() => {
  invoke.mockClear();
  vi.stubGlobal("localStorage", memory());
  vi.spyOn(document, "hasFocus").mockReturnValue(false);
});

describe("sessionState", () => {
  it("tells working, waiting, idle and done apart", async () => {
    const { sessionState } = await load();
    expect(sessionState(agent({ idle_ms: 500 }))).toBe("working");
    expect(sessionState(agent({ idle_ms: 9000 }))).toBe("idle");
    expect(sessionState(agent({ idle_ms: 9000, waiting: true }))).toBe("waiting");
    expect(sessionState(agent({ running: false }))).toBe("done");
  });
});

describe("attention flag and notifications", () => {
  it("flags a worktree that finished while another one is open, and notifies", async () => {
    const store = await load();
    store.selectWorktree(OTHER);
    await tick(store, agent({ idle_ms: 100 }));
    await tick(store, agent({ idle_ms: 9000 }));
    expect(store.attention().has(WT.toLowerCase())).toBe(true);
    expect(await notified()).toBe(1);
  });

  it("does not flag the worktree on screen", async () => {
    const store = await load();
    store.selectWorktree(WT);
    await tick(store, agent({ idle_ms: 100 }));
    await tick(store, agent({ idle_ms: 9000 }));
    expect(store.attention().has(WT.toLowerCase())).toBe(false);
  });

  it("stays quiet when the user is watching with the window focused", async () => {
    vi.spyOn(document, "hasFocus").mockReturnValue(true);
    const store = await load();
    store.selectWorktree(WT);
    await tick(store, agent({ idle_ms: 100 }));
    await tick(store, agent({ idle_ms: 9000 }));
    expect(await notified()).toBe(0);
  });

  it("clears the flag when the agent starts working again", async () => {
    const store = await load();
    store.selectWorktree(OTHER);
    await tick(store, agent({ idle_ms: 100 }));
    await tick(store, agent({ idle_ms: 9000 }));
    expect(store.attention().size).toBe(1);
    await tick(store, agent({ idle_ms: 100 }));
    expect(store.attention().size).toBe(0);
  });

  it("clears a stale flag on the worktree that is on screen", async () => {
    const store = await load();
    store.setAttention(new Set([WT.toLowerCase()]));
    store.selectWorktree(WT);
    store.setAttention(new Set([WT.toLowerCase()]));
    await tick(store, agent({ idle_ms: 9000 }));
    expect(store.attention().size).toBe(0);
  });

  it("says the agent needs an answer when it stops on a question", async () => {
    const store = await load();
    store.selectWorktree(OTHER);
    await tick(store, agent({ idle_ms: 100 }));
    await tick(store, agent({ idle_ms: 9000, waiting: true }));
    await settle();
    const call = invoke.mock.calls.find((c) => c[0] === "notify_session");
    expect(String((call?.[1] as { body: string } | undefined)?.body)).toContain("precisa da tua resposta");
  });

  it("ignores shells: only agents are flagged", async () => {
    const store = await load();
    store.selectWorktree(OTHER);
    const shell = agent({ meta: { kind: "shell", worktree: WT } });
    await tick(store, { ...shell, idle_ms: 100 });
    await tick(store, { ...shell, idle_ms: 9000 });
    expect(store.attention().size).toBe(0);
    expect(await notified()).toBe(0);
  });
});
