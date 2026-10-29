import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, render } from "@solidjs/testing-library";
import { createComponent, createRoot } from "solid-js";

const h = vi.hoisted(() => ({
  subs: new Map<string, Set<(d: Record<string, unknown>) => void>>(),
  resources: 0,
  agentStatus: 0,
  codex: 0,
  wtUsage: 0,
  agentUsage: 0,
  overlaps: 0,
}));
const emit = (name: string, d: Record<string, unknown> = {}) =>
  h.subs.get(name)?.forEach((f) => f({ event: name, ...d }));
const count = (name: string) => h.subs.get(name)?.size ?? 0;

vi.mock("./api", () => ({
  onDaemonEvent: (name: string, fn: (d: Record<string, unknown>) => void) => {
    let set = h.subs.get(name);
    if (!set) h.subs.set(name, (set = new Set()));
    set.add(fn);
    return () => void set!.delete(fn);
  },
  sessionResources: async () => {
    h.resources++;
    return { sessions: [], cores: 4 };
  },
  workerLimits: async () => ({ max_workers: 3, running: 0 }),
  killSession: async () => undefined,
  removeSession: async () => undefined,
  agentStatus: async () => {
    h.agentStatus++;
    return null;
  },
  codexLimits: async () => {
    h.codex++;
    return null;
  },
  worktreeUsage: async () => {
    h.wtUsage++;
    return [];
  },
  agentUsage: async () => {
    h.agentUsage++;
    return null;
  },
  dispatchOverlapFiles: async () => {
    h.overlaps++;
    return null;
  },
}));
vi.mock("./store", () => ({
  dismissExited: vi.fn(),
  openSession: vi.fn(),
  refreshSessions: vi.fn(async () => undefined),
  sessionState: () => "idle",
  sessions: () => [{ id: 7, running: true, label: null, meta: { agent: "claude", key: "k7", worktree: "C:/w" } }],
  activeSession: () => 7,
  projects: () => [{ id: "p", name: "p", path: "C:/p", worktrees: [{ path: "C:/p", branch: "main" }] }],
  setShowOrchestra: vi.fn(),
}));
vi.mock("./budgetStore", () => ({ budgetRows: () => [], loadBudgets: vi.fn(async () => undefined) }));
vi.mock("./Confirm", () => ({ ask: vi.fn(async () => true) }));

import { Orchestra } from "./Orchestra";
import { LimitsBar } from "./LimitsBar";
import { loadWtUsage, pollWtUsage } from "./Usage";

beforeEach(() => {
  h.subs.clear();
  h.resources = h.agentStatus = h.codex = h.wtUsage = h.agentUsage = h.overlaps = 0;
});
afterEach(() => cleanup());

describe("Orchestra reads resources on events, not on a timer", () => {
  it("reads once on open, again on session.changed / resources.changed, and unsubscribes on close", async () => {
    vi.useFakeTimers();
    const r = render(() => createComponent(Orchestra, {}));
    await vi.advanceTimersByTimeAsync(0);
    expect(h.resources).toBe(1);
    expect(vi.getTimerCount()).toBe(0);
    await vi.advanceTimersByTimeAsync(60_000);
    expect(h.resources).toBe(1);
    // a burst of both events is one reading, 2 s after the first
    emit("session.changed");
    emit("resources.changed");
    await vi.advanceTimersByTimeAsync(1999);
    expect(h.resources).toBe(1);
    await vi.advanceTimersByTimeAsync(1);
    expect(h.resources).toBe(2);
    emit("resources.changed");
    await vi.advanceTimersByTimeAsync(2000);
    expect(h.resources).toBe(3);
    r.unmount();
    expect(count("session.changed")).toBe(0);
    expect(count("resources.changed")).toBe(0);
    vi.useRealTimers();
  });
});

describe("LimitsBar refreshes on limits.changed", () => {
  it("claude status and codex limits are read at start and on limits.changed; no timer", async () => {
    vi.useFakeTimers();
    const r = render(() => createComponent(LimitsBar, {}));
    await vi.advanceTimersByTimeAsync(0);
    expect(h.agentStatus).toBe(1);
    expect(h.codex).toBe(1);
    await vi.advanceTimersByTimeAsync(120_000);
    expect(h.agentStatus).toBe(1);
    expect(h.codex).toBe(1);
    emit("limits.changed");
    emit("session.changed");
    await vi.advanceTimersByTimeAsync(300);
    expect(h.agentStatus).toBe(2);
    expect(h.codex).toBe(2);
    r.unmount();
    expect(count("limits.changed")).toBe(0);
    vi.useRealTimers();
  });
});

describe("Usage refreshes on usage.changed", () => {
  it("two callers share one subscription", async () => {
    const C = () => {
      pollWtUsage();
      pollWtUsage();
      return null;
    };
    const r = render(() => createComponent(C, {}));
    await Promise.resolve();
    expect(count("usage.changed")).toBe(1);
    expect(h.wtUsage).toBe(1);
    r.unmount();
    expect(count("usage.changed")).toBe(0);
    expect(count("session.changed")).toBe(0);
  });

  it("the worktree totals are read on usage.changed and budget.events, not on a timer", async () => {
    vi.useFakeTimers();
    createRoot((dispose) => {
      // onMount needs a component: run it through one
      const C = () => {
        pollWtUsage();
        return null;
      };
      render(() => createComponent(C, {}));
      void dispose;
    });
    await vi.advanceTimersByTimeAsync(0);
    expect(h.wtUsage).toBe(1);
    await vi.advanceTimersByTimeAsync(120_000);
    expect(h.wtUsage).toBe(1);
    emit("usage.changed");
    emit("budget.events");
    await vi.advanceTimersByTimeAsync(400);
    expect(h.wtUsage).toBe(2);
    // session.changed (about once a second) does nothing while the worktrees are the same
    for (let i = 0; i < 5; i++) {
      emit("session.changed");
      await vi.advanceTimersByTimeAsync(1000);
    }
    expect(h.wtUsage).toBe(2);
    expect(typeof loadWtUsage).toBe("function");
    vi.useRealTimers();
  });
});
