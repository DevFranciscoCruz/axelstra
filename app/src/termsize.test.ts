import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { attachWithRetry, createSizeSync, createWriteBatcher } from "./termsize";

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

  it("a grid that flickered and came back to the same size is nudged so the agent redraws", async () => {
    const { send, s } = make();
    s.push();
    await Promise.resolve();
    expect(send).toHaveBeenCalledTimes(1);
    // xterm resized twice inside the debounce and ended where it was
    s.resized();
    s.resized();
    vi.advanceTimersByTime(80);
    for (let i = 0; i < 5; i++) await Promise.resolve();
    expect(send.mock.calls.slice(1)).toEqual([[100, 29], [100, 30]]);
  });

  it("a resize that really changes the size is sent once, without a nudge", async () => {
    const { state, send, s } = make();
    s.push();
    await Promise.resolve();
    state.cols = 90;
    s.resized();
    vi.advanceTimersByTime(80);
    await Promise.resolve();
    expect(send.mock.calls.slice(1)).toEqual([[90, 30]]);
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

describe("createWriteBatcher", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("joins the chunks of one frame into a single write", () => {
    const write = vi.fn();
    const b = createWriteBatcher(write, { delay: () => 8 });
    b.push(new Uint8Array([1, 2]));
    b.push(new Uint8Array([3]));
    expect(write).not.toHaveBeenCalled();
    vi.advanceTimersByTime(8);
    expect(write).toHaveBeenCalledTimes(1);
    expect(Array.from(write.mock.calls[0][0] as Uint8Array)).toEqual([1, 2, 3]);
  });

  it("writes a big backlog at once and a flush drains early", () => {
    const write = vi.fn();
    const b = createWriteBatcher(write, { delay: () => 8, maxBytes: 4 });
    b.push(new Uint8Array(4));
    expect(write).toHaveBeenCalledTimes(1);
    b.push(new Uint8Array([9]));
    b.flush();
    expect(write).toHaveBeenCalledTimes(2);
    vi.advanceTimersByTime(50);
    expect(write).toHaveBeenCalledTimes(2);
  });

  it("waits longer while hidden and drops everything on cancel", () => {
    const write = vi.fn();
    let hidden = true;
    const b = createWriteBatcher(write, { delay: () => (hidden ? 120 : 8) });
    b.push(new Uint8Array([1]));
    vi.advanceTimersByTime(100);
    expect(write).not.toHaveBeenCalled();
    vi.advanceTimersByTime(20);
    expect(write).toHaveBeenCalledTimes(1);
    hidden = false;
    b.push(new Uint8Array([2]));
    b.cancel();
    vi.advanceTimersByTime(200);
    expect(write).toHaveBeenCalledTimes(1);
  });
});

describe("attachWithRetry", () => {
  const sleep = () => Promise.resolve();
  it("resolves with the first success", async () => {
    expect(await attachWithRetry(() => Promise.resolve("tok"), 600, { sleep })).toEqual({ ok: true, value: "tok" });
  });
  it("retries once, and the result covers the retry", async () => {
    const attempt = vi.fn().mockRejectedValueOnce(new Error("busy")).mockResolvedValue("tok");
    expect(await attachWithRetry(attempt, 600, { sleep })).toEqual({ ok: true, value: "tok" });
    expect(attempt).toHaveBeenCalledTimes(2);
  });
  it("reports the last error and fails when both tries fail", async () => {
    const onFail = vi.fn();
    expect(await attachWithRetry(() => Promise.reject(new Error("down")), 600, { sleep, onFail })).toEqual({ ok: false });
    expect(onFail).toHaveBeenCalledOnce();
  });
  it("does not retry once the view is gone", async () => {
    const attempt = vi.fn().mockRejectedValue(new Error("x"));
    expect(await attachWithRetry(attempt, 600, { sleep, gone: () => true })).toEqual({ ok: false });
    expect(attempt).toHaveBeenCalledTimes(1);
  });
});
