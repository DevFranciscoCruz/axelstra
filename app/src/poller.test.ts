import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createPoller } from "./poller";

describe("createPoller", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("ticks every ms and stops", () => {
    const fn = vi.fn();
    const p = createPoller(fn, 100);
    vi.advanceTimersByTime(350);
    expect(fn).toHaveBeenCalledTimes(3);
    p.stop();
    vi.advanceTimersByTime(500);
    expect(fn).toHaveBeenCalledTimes(3);
  });

  it("does not overlap a slow call", async () => {
    let release!: () => void;
    const fn = vi.fn(() => new Promise<void>((r) => (release = r)));
    createPoller(fn, 100);
    vi.advanceTimersByTime(500);
    expect(fn).toHaveBeenCalledTimes(1);
    release();
    await vi.advanceTimersByTimeAsync(100);
    expect(fn).toHaveBeenCalledTimes(2);
  });

  it("skips ticks while the window is hidden", () => {
    const fn = vi.fn();
    createPoller(fn, 100);
    Object.defineProperty(document, "hidden", { configurable: true, value: true });
    vi.advanceTimersByTime(300);
    expect(fn).not.toHaveBeenCalled();
    Object.defineProperty(document, "hidden", { configurable: true, value: false });
    vi.advanceTimersByTime(100);
    expect(fn).toHaveBeenCalledTimes(1);
  });

  it("a throwing or rejecting fn does not stop the poller", async () => {
    let n = 0;
    const fn = vi.fn(() => {
      if (n++ === 0) throw new Error("x");
      return Promise.reject(new Error("y"));
    });
    createPoller(fn, 100);
    await vi.advanceTimersByTimeAsync(300);
    expect(fn).toHaveBeenCalledTimes(3);
  });
});
