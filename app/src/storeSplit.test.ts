import { describe, expect, it, vi } from "vitest";

const mem = new Map<string, string>();
vi.stubGlobal("localStorage", {
  getItem: (k: string) => mem.get(k) ?? null,
  setItem: (k: string, v: string) => void mem.set(k, v),
  removeItem: (k: string) => void mem.delete(k),
});

import * as store from "./store";
import * as ui from "./store/ui";
import * as sessions from "./store/sessions";
import * as tabs from "./store/tabs";
import * as git from "./store/git";

describe("store split", () => {
  it("store.ts re-exports every topic module", () => {
    for (const mod of [ui, sessions, tabs, git]) for (const k of Object.keys(mod)) expect(store).toHaveProperty(k);
  });
  it("pins survive through createPersisted", () => {
    mem.delete("axelstra.pins");
    store.togglePin("/a/b");
    expect(store.isPinned("/a/b")).toBe(true);
    expect(JSON.parse(mem.get("axelstra.pins")!)).toHaveLength(1);
    store.togglePin("/a/b");
    expect(store.isPinned("/a/b")).toBe(false);
  });
});
