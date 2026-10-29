import { afterEach, describe, expect, it, vi } from "vitest";

const bus = vi.hoisted(() => {
  const subs = new Map<string, Set<(d: Record<string, unknown>) => void>>();
  return {
    subs,
    on: (name: string, fn: (d: Record<string, unknown>) => void) => {
      let s = subs.get(name);
      if (!s) subs.set(name, (s = new Set()));
      s.add(fn);
      return () => void s!.delete(fn);
    },
    emit: (name: string, d: Record<string, unknown> = {}) => subs.get(name)?.forEach((f) => f({ event: name, ...d })),
    count: (name: string) => subs.get(name)?.size ?? 0,
  };
});
vi.mock("./api", () => ({ onDaemonEvent: bus.on }));

import { createLive } from "./live";

const tick = () => new Promise((r) => setTimeout(r, 0));

afterEach(() => {
  bus.subs.clear();
  vi.restoreAllMocks();
});

describe("createLive", () => {
  afterEach(() => vi.useRealTimers());

  it("runs the load on each event it subscribed to and on none of the others", () => {
    const load = vi.fn();
    const live = createLive(load, ["kanban.changed", "orch.changed"]);
    bus.emit("kanban.changed");
    bus.emit("orch.changed");
    bus.emit("docker.changed");
    expect(load).toHaveBeenCalledTimes(2);
    live.stop();
  });

  it("does not load by itself: no first call, no timer", () => {
    vi.useFakeTimers();
    const load = vi.fn();
    const live = createLive(load, ["kanban.changed"]);
    vi.advanceTimersByTime(10 * 60_000);
    expect(load).not.toHaveBeenCalled();
    expect(vi.getTimerCount()).toBe(0);
    live.stop();
    vi.useRealTimers();
  });

  it("match filters by the payload", () => {
    const load = vi.fn();
    const live = createLive(load, ["kanban.changed"], { match: (d) => d.project === "p1" });
    bus.emit("kanban.changed", { project: "p2" });
    expect(load).not.toHaveBeenCalled();
    bus.emit("kanban.changed", { project: "p1" });
    expect(load).toHaveBeenCalledTimes(1);
    live.stop();
  });

  it("an event during a load makes one more run after it, never two at once", async () => {
    let release!: () => void;
    let running = 0;
    let maxRunning = 0;
    const load = vi.fn(() => {
      running++;
      maxRunning = Math.max(maxRunning, running);
      return new Promise<void>((r) => (release = () => (running--, r())));
    });
    const live = createLive(load, ["session.changed"]);
    bus.emit("session.changed");
    bus.emit("session.changed");
    bus.emit("session.changed");
    expect(load).toHaveBeenCalledTimes(1);
    release();
    await tick();
    expect(load).toHaveBeenCalledTimes(2);
    release();
    await tick();
    expect(load).toHaveBeenCalledTimes(2);
    expect(maxRunning).toBe(1);
    live.stop();
  });

  it("survives a load that throws or rejects", async () => {
    const load = vi
      .fn()
      .mockImplementationOnce(() => {
        throw new Error("x");
      })
      .mockImplementationOnce(() => Promise.reject(new Error("y")));
    const live = createLive(load, ["session.changed"]);
    bus.emit("session.changed");
    bus.emit("session.changed");
    await tick();
    bus.emit("session.changed");
    expect(load).toHaveBeenCalledTimes(3);
    live.stop();
  });

  it("runs once when the window shows again, not while it is hidden (fallback for a daemon without the event)", () => {
    const load = vi.fn();
    const live = createLive(load, ["kanban.changed"]);
    const hidden = vi.spyOn(document, "hidden", "get");
    hidden.mockReturnValue(true);
    document.dispatchEvent(new Event("visibilitychange"));
    expect(load).not.toHaveBeenCalled();
    hidden.mockReturnValue(false);
    document.dispatchEvent(new Event("visibilitychange"));
    expect(load).toHaveBeenCalledTimes(1);
    window.dispatchEvent(new Event("focus"));
    expect(load).toHaveBeenCalledTimes(2);
    live.stop();
  });

  it("onFocus: false leaves focus alone", () => {
    const load = vi.fn();
    const live = createLive(load, ["kanban.changed"], { onFocus: false });
    window.dispatchEvent(new Event("focus"));
    document.dispatchEvent(new Event("visibilitychange"));
    expect(load).not.toHaveBeenCalled();
    live.stop();
  });

  it("debounceMs: a burst gives one run after the first event, a steady stream one per window", () => {
    vi.useFakeTimers();
    const load = vi.fn();
    const live = createLive(load, ["session.changed"], { debounceMs: 400 });
    bus.emit("session.changed");
    vi.advanceTimersByTime(300);
    bus.emit("session.changed");
    bus.emit("session.changed");
    expect(load).not.toHaveBeenCalled();
    vi.advanceTimersByTime(100);
    expect(load).toHaveBeenCalledTimes(1);
    for (let i = 0; i < 10; i++) {
      bus.emit("session.changed");
      vi.advanceTimersByTime(100);
    }
    expect(load).toHaveBeenCalledTimes(3);
    live.now();
    expect(load).toHaveBeenCalledTimes(4);
    live.stop();
    expect(vi.getTimerCount()).toBe(0);
    vi.useRealTimers();
  });

  it("stop cancels a pending debounced run", () => {
    vi.useFakeTimers();
    const load = vi.fn();
    const live = createLive(load, ["session.changed"], { debounceMs: 400 });
    bus.emit("session.changed");
    live.stop();
    vi.advanceTimersByTime(1000);
    expect(load).not.toHaveBeenCalled();
    vi.useRealTimers();
  });

  it("events while the window is hidden run nothing; showing it runs once", () => {
    const load = vi.fn();
    const live = createLive(load, ["usage.changed"]);
    const hidden = vi.spyOn(document, "hidden", "get");
    hidden.mockReturnValue(true);
    bus.emit("usage.changed");
    bus.emit("usage.changed");
    expect(load).not.toHaveBeenCalled();
    hidden.mockReturnValue(false);
    document.dispatchEvent(new Event("visibilitychange"));
    expect(load).toHaveBeenCalledTimes(1);
    live.stop();
  });

  it("soft events run the load only when the signature moved since the last run", () => {
    let sig = "a";
    const load = vi.fn();
    const live = createLive(load, ["usage.changed"], { soft: { events: ["session.changed"], sig: () => sig } });
    live.now();
    expect(load).toHaveBeenCalledTimes(1);
    bus.emit("session.changed");
    bus.emit("session.changed");
    expect(load).toHaveBeenCalledTimes(1);
    sig = "b";
    bus.emit("session.changed");
    expect(load).toHaveBeenCalledTimes(2);
    bus.emit("session.changed");
    expect(load).toHaveBeenCalledTimes(2);
    bus.emit("usage.changed");
    expect(load).toHaveBeenCalledTimes(3);
    live.stop();
    expect(bus.count("session.changed")).toBe(0);
  });

  it("stop unsubscribes and ignores a run that was waiting", async () => {
    let release!: () => void;
    const load = vi.fn(() => new Promise<void>((r) => (release = r)));
    const live = createLive(load, ["session.changed"]);
    bus.emit("session.changed");
    bus.emit("session.changed");
    live.stop();
    expect(bus.count("session.changed")).toBe(0);
    release();
    await tick();
    expect(load).toHaveBeenCalledTimes(1);
    window.dispatchEvent(new Event("focus"));
    expect(load).toHaveBeenCalledTimes(1);
  });
});
