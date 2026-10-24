import { afterEach, describe, expect, it, vi } from "vitest";
import { isMirrored, syncUi } from "./uiState";
import { setMirror, writeJson, writeString } from "./persist";

class MemStorage {
  private m = new Map<string, string>();
  get length() {
    return this.m.size;
  }
  key(i: number) {
    return [...this.m.keys()][i] ?? null;
  }
  getItem(k: string) {
    return this.m.get(k) ?? null;
  }
  setItem(k: string, v: string) {
    this.m.set(k, v);
  }
}

afterEach(() => {
  setMirror(null);
  vi.useRealTimers();
});

describe("ui state mirror", () => {
  it("mirrors axelstra.* keys but not theme, terminal prefs or the update dismissal", () => {
    expect(isMirrored("axelstra.pins")).toBe(true);
    expect(isMirrored("axelstra.notifications")).toBe(true);
    for (const k of ["axelstra.theme", "axelstra.terminal", "axelstra.updateDismissed", "other.pins"]) expect(isMirrored(k)).toBe(false);
  });

  it("the daemon's copy fills a fresh profile and wins over a stale local value", async () => {
    const storage = new MemStorage();
    storage.setItem("axelstra.groupsFolded", '["old"]');
    const save = vi.fn(async () => ({}));
    const r = await syncUi({ load: async () => ({ "axelstra.pins": '["a"]', "axelstra.groupsFolded": '["new"]' }), save, storage });
    expect(storage.getItem("axelstra.pins")).toBe('["a"]');
    expect(storage.getItem("axelstra.groupsFolded")).toBe('["new"]');
    expect(r?.restored.sort()).toEqual(["axelstra.groupsFolded", "axelstra.pins"]);
    expect(save).not.toHaveBeenCalled();
  });

  it("keys only this profile has are migrated to the daemon, once", async () => {
    const storage = new MemStorage();
    storage.setItem("axelstra.pins", '["p"]');
    storage.setItem("axelstra.notifications", "[]");
    storage.setItem("axelstra.theme", "dark");
    storage.setItem("unrelated", "x");
    const save = vi.fn(async () => ({}));
    const r = await syncUi({ load: async () => ({ "axelstra.notifications": "[]" }), save, storage });
    expect(r?.migrated).toEqual(["axelstra.pins"]);
    expect(save).toHaveBeenCalledWith({ "axelstra.pins": '["p"]' });
  });

  it("later writes are sent as one debounced batch", async () => {
    vi.useFakeTimers();
    const save = vi.fn(async () => ({}));
    await syncUi({ load: async () => ({}), save, storage: new MemStorage(), delayMs: 100 });
    writeJson("axelstra.pins", ["a"]);
    writeString("axelstra.groupsFolded", "[]");
    writeString("axelstra.theme", "dark");
    expect(save).not.toHaveBeenCalled();
    vi.advanceTimersByTime(100);
    expect(save).toHaveBeenCalledTimes(1);
    expect(save).toHaveBeenCalledWith({ "axelstra.pins": '["a"]', "axelstra.groupsFolded": "[]" });
  });

  it("a daemon that cannot answer leaves the app on localStorage and never throws", async () => {
    const save = vi.fn(async () => ({}));
    expect(await syncUi({ load: async () => Promise.reject(new Error("unknown method")), save, storage: new MemStorage() })).toBeNull();
    writeString("axelstra.pins", "[]");
    expect(save).not.toHaveBeenCalled();
  });
});
