import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createSizeSync } from "./termsize";

describe("createSizeSync", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  const make = (over: Partial<Parameters<typeof createSizeSync>[0]> = {}) => {
    const state = { cols: 100, rows: 30, visible: true };
    const send = vi.fn(() => Promise.resolve());
    const s = createSizeSync({ size: () => state, visible: () => state.visible, gone: () => false, send, ...over });
    return { state, send, s };
  };

  it("sends the size once and not again while it is unchanged", async () => {
    const { send, s } = make();
    s.push();
    await Promise.resolve();
    s.push();
    expect(send).toHaveBeenCalledTimes(1);
    expect(send).toHaveBeenCalledWith(100, 30);
  });

  it("never sends while hidden, and sends when shown", async () => {
    const { state, send, s } = make();
    state.visible = false;
    s.push();
    expect(send).not.toHaveBeenCalled();
    state.visible = true;
    s.push();
    expect(send).toHaveBeenCalledTimes(1);
  });

  it("retries after a failure", async () => {
    const send = vi.fn().mockRejectedValueOnce(new Error("down")).mockResolvedValue(undefined);
    const { s } = make({ send });
    s.push();
    await Promise.resolve();
    await Promise.resolve();
    vi.advanceTimersByTime(500);
    expect(send).toHaveBeenCalledTimes(2);
  });

  it("sends only the last size of a drag", () => {
    const { state, send, s } = make();
    for (const c of [90, 95, 99]) {
      state.cols = c;
      s.resized();
      vi.advanceTimersByTime(20);
    }
    vi.advanceTimersByTime(100);
    expect(send).toHaveBeenCalledTimes(1);
    expect(send).toHaveBeenCalledWith(99, 30);
  });

  it("resends after forget (new subscription)", async () => {
    const { send, s } = make();
    s.push();
    await Promise.resolve();
    await Promise.resolve();
    s.forget();
    s.push();
    expect(send).toHaveBeenCalledTimes(2);
  });
});
