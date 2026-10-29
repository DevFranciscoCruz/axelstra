import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { SessionInfo } from "./api";

const invoke = vi.hoisted(() =>
  vi.fn(async (cmd: string, _args?: unknown) => (cmd === "notify_session" ? true : undefined)),
);
const sessionsNow = vi.hoisted(() => ({ list: [] as unknown[], calls: 0, gate: null as Promise<void> | null }));
const projectsNow = vi.hoisted(() => ({ list: [] as unknown[] }));
const changesNow = vi.hoisted(() => ({ answer: (_wt: string): Promise<unknown> => Promise.resolve(null) }));

vi.mock("@tauri-apps/api/core", () => ({ invoke }));
vi.mock("./Confirm", () => ({ ask: vi.fn(async () => true) }));
vi.mock("./api", () => ({
  listSessions: async () => {
    sessionsNow.calls++;
    await sessionsNow.gate;
    return sessionsNow.list;
  },
  listProjects: async () => projectsNow.list,
  worktreeStatus: async () => [],
  ghListBg: async () => [],
  gitChanges: (wt: string) => changesNow.answer(wt),
  flowTasks: async () => [],
  worktreeOverview: async () => [],
  apiContext: { projects: () => [], terse: () => undefined },
  call: async () => ({}),
  callBg: async () => ({}),
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
  return {
    getItem: (k: string) => m.get(k) ?? null,
    setItem: (k: string, v: string) => void m.set(k, v),
    removeItem: (k: string) => void m.delete(k),
    clear: () => m.clear(),
  };
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
  // the store ignores the first seconds after start and short bursts of work: a clock that jumps 40 s per read gets past both
  let clock = 0;
  beforeEach(() => {
    clock = 0;
    vi.spyOn(Date, "now").mockImplementation(() => (clock += 40000));
  });
  afterEach(() => vi.restoreAllMocks());

  it("flags a worktree that finished while another one is open, without a system notification", async () => {
    const store = await load();
    store.selectWorktree(OTHER);
    await tick(store, agent({ idle_ms: 100 }));
    await tick(store, agent({ idle_ms: 9000 }));
    expect(store.attention().has(WT.toLowerCase())).toBe(true);
    expect(await notified()).toBe(0);
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

  it("does not notify for a restored session that only looked busy for a moment", async () => {
    vi.spyOn(Date, "now").mockImplementation(() => (clock += 500));
    const store = await load();
    store.selectWorktree(OTHER);
    await tick(store, agent({ idle_ms: 100 }));
    await tick(store, agent({ idle_ms: 9000 }));
    await settle();
    expect(
      invoke.mock.calls.find((c) => c[0] === "notify_session" && (c[1] as { waiting: boolean }).waiting),
    ).toBeUndefined();
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

  it("says what the agent asked when it sends a wait event", async () => {
    const store = await load();
    store.selectWorktree(OTHER);
    await tick(store, agent({ idle_ms: 100 }));
    await tick(store, agent({ idle_ms: 9000, waiting: true, wait: { kind: "question", text: "Postgres ou SQLite?" } }));
    await settle();
    const call = invoke.mock.calls.find((c) => c[0] === "notify_session");
    expect(String((call?.[1] as { body: string } | undefined)?.body)).toContain("Postgres ou SQLite?");
  });

  it("tells the host when the agent is waiting on the user", async () => {
    const store = await load();
    store.selectWorktree(OTHER);
    await tick(store, agent({ idle_ms: 100 }));
    await tick(store, agent({ idle_ms: 9000, waiting: true, wait: { kind: "question", text: "Postgres ou SQLite?" } }));
    await settle();
    expect((invoke.mock.calls.find((c) => c[0] === "notify_session")?.[1] as { waiting: boolean }).waiting).toBe(true);
  });

  it("a waiting flag without an event is not announced as waiting", async () => {
    const store = await load();
    store.selectWorktree(OTHER);
    await tick(store, agent({ idle_ms: 100 }));
    await tick(store, agent({ idle_ms: 9000, waiting: true }));
    await settle();
    expect(
      invoke.mock.calls.find((c) => c[0] === "notify_session" && (c[1] as { waiting: boolean }).waiting),
    ).toBeUndefined();
  });

  it("an agent that just went quiet raises no system notification at all", async () => {
    const store = await load();
    store.selectWorktree(OTHER);
    await tick(store, agent({ idle_ms: 100 }));
    await tick(store, agent({ idle_ms: 9000 }));
    await settle();
    expect(invoke.mock.calls.find((c) => c[0] === "notify_session")).toBeUndefined();
  });

  const stall = { minutes: 5, code: "stall.quiet", params: {}, text: "quiet for 5 min" };

  it("a stall nobody handles (`stalled`) alerts the user once", async () => {
    const store = await load();
    store.selectWorktree(OTHER);
    await tick(store, agent({ idle_ms: 9000, stalled: true, stall }));
    await tick(store, agent({ idle_ms: 9000, stalled: true, stall }));
    expect(await notified()).toBe(1);
  });

  it("a stall a parent agent handles (`stall_parent`) is not an alert", async () => {
    const store = await load();
    store.selectWorktree(OTHER);
    await tick(store, agent({ idle_ms: 9000, stalled: false, stall_parent: true, stall }));
    expect(await notified()).toBe(0);
    expect(store.attention().size).toBe(0);
  });

  it("do not disturb silences the system notification of a stall", async () => {
    const store = await load();
    const prefs = await import("./notifyPrefs");
    prefs.setNotifyPref("dnd", true);
    store.selectWorktree(OTHER);
    await tick(store, agent({ idle_ms: 9000, stalled: true, stall }));
    expect(await notified()).toBe(0);
  });

  it("a finished top-level agent notifies only when the Finished category is on", async () => {
    const store = await load();
    const prefs = await import("./notifyPrefs");
    store.selectWorktree(OTHER);
    await tick(store, agent({ idle_ms: 100 }));
    await tick(store, agent({ running: false }));
    expect(await notified()).toBe(0);
    prefs.setNotifyPref("done", true);
    await tick(store, agent({ idle_ms: 100 }));
    await tick(store, agent({ running: false }));
    expect(await notified()).toBe(1);
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

describe("keepSame", () => {
  it("reuses the old object when only ignored fields moved", async () => {
    const { keepSame } = await load();
    const a = { id: 1, idle: 5, name: "x" };
    const out = keepSame(
      [a],
      [{ id: 1, idle: 900, name: "x" }],
      (x) => x.id,
      (x) => ({ ...x, idle: 0 }),
    );
    expect(out[0]).toBe(a);
    const changed = keepSame(
      [a],
      [{ id: 1, idle: 900, name: "y" }],
      (x) => x.id,
      (x) => ({ ...x, idle: 0 }),
    );
    expect(changed[0]).not.toBe(a);
  });

  it("keeps session objects across polls while idle_ms only ticks", async () => {
    const store = await load();
    await tick(store, agent({ idle_ms: 10_000 }));
    const first = store.sessions()[0];
    await tick(store, agent({ idle_ms: 15_000 }));
    expect(store.sessions()[0]).toBe(first);
    // crossing into "working" is a real change
    await tick(store, agent({ idle_ms: 100 }));
    expect(store.sessions()[0]).not.toBe(first);
  });
});

describe("refreshSessions", () => {
  it("shares one trailing poll between calls that arrive during a poll", async () => {
    const store = await load();
    sessionsNow.calls = 0;
    let open!: () => void;
    sessionsNow.gate = new Promise<void>((r) => (open = r));
    const calls = [store.refreshSessions(), store.refreshSessions(), store.refreshSessions()];
    sessionsNow.gate = null;
    open();
    await Promise.all(calls);
    expect(sessionsNow.calls).toBe(2);
  });
});

describe("removed sessions", () => {
  it("forgets the state of a session that is gone, so a reused id starts fresh", async () => {
    const store = await load();
    store.selectWorktree(OTHER);
    await tick(store, agent({ idle_ms: 100 }));
    sessionsNow.list = [];
    await store.refreshSessions();
    // same id comes back already quiet: it was never seen working, so nothing to flag
    await tick(store, agent({ idle_ms: 9000 }));
    expect(store.attention().size).toBe(0);
  });
});

describe("refreshChanges", () => {
  it("drops an answer for a worktree that is no longer selected", async () => {
    const wts = [WT, OTHER].map((path) => ({ path, branch: "b", head: "h", is_main: false }));
    projectsNow.list = [{ id: "p", name: "p", path: "C:/work", worktrees: wts }];
    const store = await load();
    await store.refreshProjects();
    const slow: Record<string, (v: unknown) => void> = {};
    changesNow.answer = (wt) => new Promise((r) => (slow[wt] = r));
    store.selectWorktree(WT);
    const first = store.refreshChanges();
    store.selectWorktree(OTHER);
    const second = store.refreshChanges();
    slow[OTHER]({ files: [{ path: "b.ts" }] });
    await second;
    slow[WT]({ files: [{ path: "a.ts" }] });
    await first;
    expect((store.changes() as unknown as { files: { path: string }[] }).files[0].path).toBe("b.ts");
    projectsNow.list = [];
    changesNow.answer = () => Promise.resolve(null);
  });
});

describe("main view", () => {
  it("showView clears every overlay and openFileAt remembers the line", async () => {
    const store = await load();
    store.setOpenDiff("a.ts");
    store.setShowKanban(true);
    store.openFileAt("src/x.ts", 42);
    expect(store.openFile()).toBe("src/x.ts");
    expect(store.fileLine()).toBe(42);
    expect(store.openDiff()).toBeNull();
    expect(store.showKanban()).toBe(false);
    store.showView();
    expect(store.openFile()).toBeNull();
    expect(store.fileLine()).toBeNull();
  });
});

describe("tab order", () => {
  it("puts dragged tabs first and the rest after in natural order", async () => {
    const { orderTabs, moveBefore } = await load();
    const list = [{ id: 1 }, { id: 2 }, { id: 3 }];
    expect(orderTabs(list, []).map((x) => x.id)).toEqual([1, 2, 3]);
    expect(orderTabs(list, [3, 1]).map((x) => x.id)).toEqual([3, 1, 2]);
    expect(moveBefore([], [1, 2, 3], 3, 1)).toEqual([3, 1, 2]);
    expect(moveBefore([], [1, 2, 3], 1, null)).toEqual([2, 3, 1]);
    expect(moveBefore([9], [1, 2], 2, 1)).toEqual([9, 2, 1]);
    expect(moveBefore([4], [1, 2], 1, 99)).toEqual([4]);
  });

  it("reopens the most recently closed tab first", async () => {
    const store = await load();
    store.rememberClosed(agent({ id: 1, label: "um" }));
    store.rememberClosed(agent({ id: 2, label: "dois" }));
    expect(store.popClosed()?.label).toBe("dois");
    expect(store.popClosed()?.label).toBe("um");
    expect(store.popClosed()).toBeNull();
  });

  it("a renamed tab shows its new name, an empty name restores the original", async () => {
    const store = await load();
    const s = agent({ id: 7, label: "claude" });
    store.renameTab(7, "  api  ");
    expect(store.tabLabel(s)).toBe("api");
    store.renameTab(7, "");
    expect(store.tabLabel(s)).toBe("claude");
  });
});

describe("project preferences", () => {
  it("a project can pick its own agent and fall back to the global one", async () => {
    const store = await load();
    expect(store.projectAgent("p1")).toBe("claude");
    store.setProjectPref("p1", { agent: "codex" });
    expect(store.projectAgent("p1")).toBe("codex");
    expect(store.projectAgent("p2")).toBe("claude");
    store.setProjectPref("p1", { agent: undefined });
    expect(store.projectAgent("p1")).toBe("claude");
    expect(store.projectPrefsOf("p1")).toEqual({});
  });

  it("remembers the reply style per project", async () => {
    const store = await load();
    store.setProjectPref("p1", { terse: "tiny" });
    expect(store.projectTerse("p1")).toBe("tiny");
    expect(store.projectTerse("p2")).toBeUndefined();
  });
});

describe("friendlyError", () => {
  it("explains a known failure and keeps the original text", async () => {
    const { friendlyError } = await load();
    const out = friendlyError("fatal: not a git repository (or any parent)");
    expect(out.startsWith("Esta pasta não é um repositório git.")).toBe(true);
    expect(out).toContain("(fatal: not a git repository (or any parent))");
  });
  it("leaves an unknown message alone", async () => {
    const { friendlyError } = await load();
    expect(friendlyError("something odd")).toBe("something odd");
  });
  it("cuts a long original at 600 characters", async () => {
    const { friendlyError } = await load();
    const out = friendlyError("not a git repository " + "x".repeat(700));
    expect(out).toContain("…)");
    expect(out.length).toBeLessThan(760);
  });
});

describe("the view only moves by the user's own action", () => {
  it("sessions and finished agents of another project do not change the selection", async () => {
    const store = await load();
    store.selectWorktree(WT);
    store.activate(1, "user");
    // an agent in another project works, finishes and a new session appears there
    await tick(store, agent({ id: 1, meta: { kind: "agent", agent: "claude", worktree: WT } }));
    sessionsNow.list = [
      agent({ id: 1 }),
      agent({ id: 2, idle_ms: 0, meta: { kind: "agent", agent: "claude", worktree: OTHER } }),
    ];
    await store.refreshSessions();
    sessionsNow.list = [
      agent({ id: 1 }),
      agent({ id: 2, running: false, meta: { kind: "agent", agent: "claude", worktree: OTHER } }),
      agent({ id: 3, meta: { kind: "agent", worktree: OTHER } }),
    ];
    await store.refreshSessions();
    await settle();
    expect(store.selectedWorktree()).toBe(WT);
    expect(store.activeSession()).toBe(1);
  });
  it("a flow that finishes after the user went elsewhere does not pull the view back", async () => {
    const store = await load();
    store.selectWorktree(WT);
    const stamp = store.navStamp();
    store.selectWorktree(OTHER); // the user clicked another worktree while something was being created
    store.selectIfStill(stamp, "C:/work/proj-new");
    store.openIfStill(stamp, "C:/work/proj-new", 9);
    expect(store.selectedWorktree()).toBe(OTHER);
    expect(store.activeSession()).toBe(null);
  });
  it("a flow the user did not interrupt still opens its result", async () => {
    const store = await load();
    store.selectWorktree(WT);
    const stamp = store.navStamp();
    store.selectIfStill(stamp, "C:/work/proj-new");
    expect(store.selectedWorktree()).toBe("C:/work/proj-new");
    const s2 = store.navStamp();
    store.openIfStill(s2, "C:/work/proj-b", 9);
    expect(store.selectedWorktree()).toBe("C:/work/proj-b");
    expect(store.activeSession()).toBe(9);
  });
});
