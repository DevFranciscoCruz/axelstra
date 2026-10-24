import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, render } from "@solidjs/testing-library";
import { createComponent } from "solid-js";

const h = vi.hoisted(() => ({
  running: true,
  statusCalls: 0,
  containerCalls: 0,
  fail: false,
}));

vi.mock("./api", () => ({
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
});
afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

describe("DockerView polling", () => {
  it("polls at 3 s while docker answers and stops when unmounted", async () => {
    const r = render(() => createComponent(DockerView, {}));
    await settle();
    expect(h.statusCalls).toBe(1);
    await vi.advanceTimersByTimeAsync(3000);
    expect(h.statusCalls).toBe(2);
    expect(h.containerCalls).toBe(2);
    r.unmount();
    await vi.advanceTimersByTimeAsync(30000);
    expect(h.statusCalls).toBe(2);
  });

  it("slows to 10 s while docker is not running", async () => {
    h.running = false;
    const r = render(() => createComponent(DockerView, {}));
    await settle();
    expect(h.statusCalls).toBe(1);
    await vi.advanceTimersByTimeAsync(3000);
    expect(h.statusCalls).toBe(1);
    await vi.advanceTimersByTimeAsync(7000);
    expect(h.statusCalls).toBe(2);
    expect(h.containerCalls).toBe(0);
    r.unmount();
  });

  it("keeps polling after a failed call", async () => {
    h.fail = true;
    const r = render(() => createComponent(DockerView, {}));
    await settle();
    h.fail = false;
    h.running = true;
    await vi.advanceTimersByTimeAsync(10000);
    expect(h.statusCalls).toBeGreaterThanOrEqual(2);
    r.unmount();
  });
});
