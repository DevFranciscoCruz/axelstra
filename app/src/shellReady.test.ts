import { describe, expect, it, vi } from "vitest";
import { waitShellReady } from "./shellReady";

const setup = () => {
  let fire = () => {};
  let deadline = () => {};
  const off = vi.fn();
  const cancel = vi.fn();
  return {
    changes: (fn: () => void) => ((fire = fn), off),
    after: (fn: () => void) => ((deadline = fn), cancel),
    emit: () => fire(),
    expire: () => deadline(),
    off,
    cancel,
  };
};
const tick = () => new Promise((r) => setTimeout(r, 0));

describe("waitShellReady", () => {
  it("is ready when the session goes quiet and the screen has the prompt", async () => {
    const t = setup();
    const read = vi.fn(async () => ["PS C:\\w>"]);
    const p = waitShellReady(read, t);
    expect(read).not.toHaveBeenCalled();
    t.emit();
    expect(await p).toBe(true);
    expect(t.off).toHaveBeenCalled();
    expect(t.cancel).toHaveBeenCalled();
  });

  it("an empty screen keeps waiting for the next change", async () => {
    const t = setup();
    const screens = [[""], ["$ "]];
    let i = 0;
    const p = waitShellReady(async () => screens[Math.min(i++, 1)], t);
    t.emit();
    await tick();
    t.emit();
    expect(await p).toBe(true);
    expect(i).toBe(2);
  });

  it("gives up at the deadline when nothing is ever printed", async () => {
    const t = setup();
    const p = waitShellReady(async () => [""], t);
    t.emit();
    await tick();
    t.expire();
    expect(await p).toBe(false);
    expect(t.off).toHaveBeenCalled();
  });

  it("survives a read that throws", async () => {
    const t = setup();
    let n = 0;
    const p = waitShellReady(async () => {
      if (n++ < 1) throw new Error("no session");
      return ["$"];
    }, t);
    t.emit();
    await tick();
    t.emit();
    expect(await p).toBe(true);
  });

  it("leaves no timer behind when a change arrives before the deadline", async () => {
    vi.useFakeTimers();
    const p = waitShellReady(async () => ["$"], { changes: (fn) => (queueMicrotask(fn), () => {}) });
    await vi.advanceTimersByTimeAsync(0);
    expect(await p).toBe(true);
    expect(vi.getTimerCount()).toBe(0);
    vi.useRealTimers();
  });
});
