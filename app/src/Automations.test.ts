import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, render } from "@solidjs/testing-library";
import { createComponent } from "solid-js";

const h = vi.hoisted(() => ({
  listCalls: 0,
  subs: new Map<string, Set<(d: Record<string, unknown>) => void>>(),
  items: [] as { id: string; name: string }[],
}));
const emit = (name: string) => h.subs.get(name)?.forEach((f) => f({ event: name }));
const settle = async () => {
  for (let i = 0; i < 10; i++) await Promise.resolve();
};

vi.mock("./api", () => ({
  onDaemonEvent: (name: string, fn: (d: Record<string, unknown>) => void) => {
    let set = h.subs.get(name);
    if (!set) h.subs.set(name, (set = new Set()));
    set.add(fn);
    return () => void set!.delete(fn);
  },
  listAutomations: vi.fn(async () => {
    h.listCalls++;
    return h.items.map((a) => ({
      ...a,
      project: "p1",
      agent: "claude",
      prompt: "x",
      schedule: { kind: "daily", time: "09:00" },
      enabled: true,
      last_run: 0,
    }));
  }),
  agentsList: vi.fn(async () => []),
  agentOptions: vi.fn(() => []),
  agentWarning: vi.fn(() => null),
  saveAutomation: vi.fn(),
  deleteAutomation: vi.fn(),
  runAutomation: vi.fn(),
}));
vi.mock("./store", () => ({
  projects: () => [{ id: "p1", name: "Proj" }],
  refreshProjects: vi.fn(async () => undefined),
  refreshSessions: vi.fn(async () => undefined),
  navStamp: () => 0,
  selectIfStill: vi.fn(),
  setError: vi.fn(),
  setShowAutomations: vi.fn(),
  toast: vi.fn(),
}));
vi.mock("./Confirm", () => ({ ask: vi.fn(async () => true), askDiscard: vi.fn(async () => true) }));
vi.mock("./Modal", () => ({ Modal: (p: { children: unknown }) => p.children }));

import { Automations } from "./Automations";

beforeEach(() => {
  vi.useFakeTimers();
  h.listCalls = 0;
  h.items = [];
  h.subs.clear();
});
afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

describe("Automations follows automations.changed", () => {
  it("loads once, reloads on each automations.changed, sets no timer and unsubscribes on close", async () => {
    const r = render(() => createComponent(Automations, {}));
    await settle();
    expect(h.listCalls).toBe(1);
    expect(h.subs.get("automations.changed")?.size).toBe(1);

    h.items = [{ id: "a1", name: "Nightly sweep" }];
    emit("automations.changed");
    await settle();
    expect(h.listCalls).toBe(2);
    expect(r.container.textContent).toContain("Nightly sweep");

    // nothing reloads by itself
    await vi.advanceTimersByTimeAsync(10 * 60_000);
    expect(h.listCalls).toBe(2);
    expect(vi.getTimerCount()).toBe(0);

    r.unmount();
    expect(h.subs.get("automations.changed")?.size).toBe(0);
    emit("automations.changed");
    await settle();
    expect(h.listCalls).toBe(2);
  });
});
