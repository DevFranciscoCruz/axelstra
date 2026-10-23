import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { hoverIntent, keepInside } from "./hoverIntent";

describe("hoverIntent", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("opens after the delay and closes after the leave delay", () => {
    const set = vi.fn();
    const h = hoverIntent(set);
    h.enter();
    vi.advanceTimersByTime(249);
    expect(set).not.toHaveBeenCalled();
    vi.advanceTimersByTime(1);
    expect(set).toHaveBeenLastCalledWith(true);
    h.leave();
    vi.advanceTimersByTime(150);
    expect(set).toHaveBeenLastCalledWith(false);
  });

  it("returning to the chip cancels the pending close", () => {
    const set = vi.fn();
    const h = hoverIntent(set);
    h.leave();
    vi.advanceTimersByTime(100);
    h.enter();
    vi.advanceTimersByTime(150);
    expect(set).not.toHaveBeenCalled();
  });

  it("cancel drops the pending change", () => {
    const set = vi.fn();
    const h = hoverIntent(set);
    h.enter();
    h.cancel();
    vi.advanceTimersByTime(1000);
    expect(set).not.toHaveBeenCalled();
  });
});

describe("keepInside", () => {
  it("shifts left only when the box overflows", () => {
    expect(keepInside(500, 1000)).toBe(0);
    expect(keepInside(1000, 1000)).toBe(-8);
    expect(keepInside(1100, 1000)).toBe(-108);
  });
});
