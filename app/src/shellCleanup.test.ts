import { afterEach, describe, expect, it, vi } from "vitest";

const handlers = new Map<string, (e: { payload: unknown }) => void>();
const unlistens: ReturnType<typeof vi.fn>[] = [];
vi.mock("@tauri-apps/api/event", () => ({
  listen: async (name: string, cb: (e: { payload: unknown }) => void) => {
    handlers.set(name, cb);
    const u = vi.fn();
    unlistens.push(u);
    return u;
  },
}));

import { onTauriEvent } from "./tauriEvents";
import { startDrag } from "./dragResize";

afterEach(() => {
  handlers.clear();
  unlistens.length = 0;
  document.body.className = "";
});

describe("onTauriEvent", () => {
  it("delivers the payload and unlistens on stop", async () => {
    const cb = vi.fn();
    const stop = onTauriEvent("x", cb);
    await new Promise((r) => setTimeout(r, 0));
    handlers.get("x")!({ payload: 7 });
    expect(cb).toHaveBeenCalledWith(7);
    stop();
    expect(unlistens[0]).toHaveBeenCalledTimes(1);
  });
  it("a stop that runs before listen resolved still unlistens (no leak)", async () => {
    const stop = onTauriEvent("y", () => {});
    stop();
    await new Promise((r) => setTimeout(r, 0));
    expect(unlistens[0]).toHaveBeenCalledTimes(1);
  });
});

describe("startDrag", () => {
  const move = (buttons: number) => window.dispatchEvent(new MouseEvent("mousemove", { buttons, clientX: 5 }));
  it("follows moves with a button down and ends on mouseup", () => {
    const on = vi.fn();
    startDrag(on);
    expect(document.body.classList.contains("resizing")).toBe(true);
    move(1);
    expect(on).toHaveBeenCalledTimes(1);
    window.dispatchEvent(new MouseEvent("mouseup"));
    move(1);
    expect(on).toHaveBeenCalledTimes(1);
    expect(document.body.classList.contains("resizing")).toBe(false);
  });
  it("ends when the mouseup was lost outside the window (a move with no button) and on blur", () => {
    const a = vi.fn();
    startDrag(a);
    move(0);
    move(1);
    expect(a).not.toHaveBeenCalled();
    expect(document.body.classList.contains("resizing")).toBe(false);
    const b = vi.fn();
    startDrag(b);
    window.dispatchEvent(new Event("blur"));
    move(1);
    expect(b).not.toHaveBeenCalled();
  });
  it("the returned stop ends it early", () => {
    const on = vi.fn();
    const stop = startDrag(on);
    stop();
    move(1);
    expect(on).not.toHaveBeenCalled();
  });
});
