import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const memory = () => {
  const m = new Map<string, string>();
  return {
    getItem: (k: string) => m.get(k) ?? null,
    setItem: (k: string, v: string) => void m.set(k, v),
    removeItem: (k: string) => void m.delete(k),
    clear: () => m.clear(),
  };
};
const load = async () => {
  vi.resetModules();
  return await import("./toasts");
};

beforeEach(() => {
  vi.stubGlobal("localStorage", memory());
  vi.useFakeTimers();
});
afterEach(() => vi.useRealTimers());

describe("toasts", () => {
  it("notices close by themselves, errors and warnings stay", async () => {
    const t = await load();
    t.toast({ text: "info" });
    t.toast({ text: "bad", kind: "error" });
    t.toast({ text: "careful", kind: "warn" });
    vi.advanceTimersByTime(t.NOTICE_TTL_MS + 1);
    expect(t.toasts().map((x) => x.text)).toEqual(["bad", "careful"]);
  });

  it("pausing keeps a notice open, resuming lets it finish", async () => {
    const t = await load();
    const id = t.toast({ text: "info" });
    vi.advanceTimersByTime(5000);
    t.pauseToast(id);
    vi.advanceTimersByTime(60_000);
    expect(t.toasts()).toHaveLength(1);
    t.resumeToast(id);
    vi.advanceTimersByTime(3900);
    expect(t.toasts()).toHaveLength(1);
    vi.advanceTimersByTime(200);
    expect(t.toasts()).toHaveLength(0);
  });

  it("the same key replaces the toast and restarts its timer; other keys stack", async () => {
    const t = await load();
    t.toast({ text: "a", key: "k" });
    vi.advanceTimersByTime(8000);
    t.toast({ text: "b", key: "k" });
    vi.advanceTimersByTime(8000);
    expect(t.toasts().map((x) => x.text)).toEqual(["b"]);
    t.toast({ text: "c", key: "other" });
    expect(t.toasts()).toHaveLength(2);
  });

  it("keeps at most MAX_TOASTS, dropping the oldest", async () => {
    const t = await load();
    for (let i = 0; i < t.MAX_TOASTS + 3; i++) t.toast({ text: `e${i}`, kind: "error" });
    expect(t.toasts()).toHaveLength(t.MAX_TOASTS);
    expect(t.toasts()[0].text).toBe("e3");
  });

  it("dismissPrefix and dismissKey close the matching ones", async () => {
    const t = await load();
    t.toast({ text: "a", kind: "error", key: "action:a" });
    t.toast({ text: "b", kind: "error", key: "action:b" });
    t.toast({ text: "c", kind: "error", key: "poll:x" });
    t.dismissPrefix("action:");
    expect(t.toasts().map((x) => x.key)).toEqual(["poll:x"]);
    t.dismissKey("poll:x");
    expect(t.toasts()).toHaveLength(0);
  });

  it("an action toast carries its button", async () => {
    const t = await load();
    const run = vi.fn();
    t.toast({ text: "Apagado", action: { label: "Desfazer", run } });
    t.toasts()[0].action!.run();
    expect(run).toHaveBeenCalledOnce();
  });
});
