import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, render } from "@solidjs/testing-library";
import { createComponent } from "solid-js";

const h = vi.hoisted(() => ({
  running: true,
  statusCalls: 0,
  containerCalls: 0,
  fail: false,
}));

const bus = vi.hoisted(() => {
  const subs = new Set<() => void>();
  return { subs, emit: () => subs.forEach((f) => f()) };
});

vi.mock("./api", () => ({
  onDaemonEvent: (name: string, fn: () => void) => {
    if (name !== "docker.changed") return () => {};
    bus.subs.add(fn);
    return () => void bus.subs.delete(fn);
  },
  dockerStatus: vi.fn(async () => {
    h.statusCalls++;
    if (h.fail) throw new Error("daemon busy");
    return { running: h.running };
  }),
  dockerContainers: vi.fn(async () => {
    h.containerCalls++;
    return { containers: [], compose: null };
  }),
  dockerStats: vi.fn(async () => []),
  dockerImages: vi.fn(async () => []),
  dockerVolumes: vi.fn(async () => []),
  dockerNetworks: vi.fn(async () => []),
  dockerAction: vi.fn(async () => undefined),
}));
vi.mock("./store", () => ({
  findWorktree: () => null,
  keepSame: (_prev: unknown, next: unknown) => next,
  openSession: vi.fn(),
  refreshSessions: vi.fn(async () => undefined),
  selectedWorktree: () => null,
  sessions: () => [],
  setError: vi.fn(),
  setShowDocker: vi.fn(),
  showDocker: () => true,
  showNotice: vi.fn(),
}));
vi.mock("./Confirm", () => ({ ask: vi.fn(async () => true) }));
vi.mock("./docker/Detail", () => ({ Detail: () => null }));
vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: vi.fn() }));

import { DockerView } from "./Docker";

const settle = async () => {
  for (let i = 0; i < 10; i++) await Promise.resolve();
};

beforeEach(() => {
  vi.useFakeTimers();
  h.running = true;
  h.fail = false;
  h.statusCalls = 0;
  h.containerCalls = 0;
  bus.subs.clear();
});
afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

describe("DockerView follows docker.changed", () => {
  it("loads once on open, again on each docker.changed, and sets no timer", async () => {
    const r = render(() => createComponent(DockerView, {}));
    await settle();
    expect(h.statusCalls).toBe(1);
    expect(vi.getTimerCount()).toBe(0);
    await vi.advanceTimersByTimeAsync(60000);
    expect(h.statusCalls).toBe(1);
    bus.emit();
    await settle();
    expect(h.statusCalls).toBe(2);
    expect(h.containerCalls).toBe(2);
    r.unmount();
  });

  it("stops listening when unmounted", async () => {
    const r = render(() => createComponent(DockerView, {}));
    await settle();
    expect(bus.subs.size).toBe(1);
    r.unmount();
    expect(bus.subs.size).toBe(0);
    bus.emit();
    await settle();
    expect(h.statusCalls).toBe(1);
  });

  it("does not ask for containers while docker is not running, and picks it up when docker.changed says it started", async () => {
    h.running = false;
    const r = render(() => createComponent(DockerView, {}));
    await settle();
    expect(h.statusCalls).toBe(1);
    expect(h.containerCalls).toBe(0);
    h.running = true;
    bus.emit();
    await settle();
    expect(h.statusCalls).toBe(2);
    expect(h.containerCalls).toBe(1);
    r.unmount();
  });

  it("recovers after a failed call on the next event", async () => {
    h.fail = true;
    const r = render(() => createComponent(DockerView, {}));
    await settle();
    h.fail = false;
    bus.emit();
    await settle();
    expect(h.statusCalls).toBe(2);
    expect(h.containerCalls).toBe(1);
    r.unmount();
  });
});
