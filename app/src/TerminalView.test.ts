import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, render } from "@solidjs/testing-library";
import { createComponent } from "solid-js";

const h = vi.hoisted(() => ({
  invoke: vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined),
  terms: [] as { disposed: boolean; written: string[] }[],
  dragListeners: 0,
  dragUnlistens: 0,
  dragHandler: null as null | ((ev: unknown) => void),
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: h.invoke,
  Channel: class {
    onmessage: unknown;
  },
}));
vi.mock("@tauri-apps/api/webview", () => ({
  getCurrentWebview: () => ({
    onDragDropEvent: async (cb: (ev: unknown) => void) => {
      h.dragListeners++;
      h.dragHandler = cb;
      return () => void h.dragUnlistens++;
    },
  }),
}));
vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: vi.fn() }));
vi.mock("@xterm/xterm", () => ({
  Terminal: class {
    cols = 80;
    rows = 24;
    options: Record<string, unknown> = {};
    unicode = { activeVersion: "" };
    buffer = { active: { getLine: () => undefined } };
    state = { disposed: false, written: [] as string[] };
    constructor() {
      h.terms.push(this.state);
    }
    loadAddon() {}
    open() {}
    registerLinkProvider() {}
    attachCustomKeyEventHandler() {}
    onData() {}
    onResize() {}
    write(d: string) {
      this.state.written.push(String(d));
    }
    paste() {}
    focus() {}
    refresh() {}
    clearTextureAtlas() {}
    hasSelection() {
      return false;
    }
    getSelection() {
      return "";
    }
    clearSelection() {}
    dispose() {
      this.state.disposed = true;
    }
  },
}));
vi.mock("@xterm/addon-fit", () => ({ FitAddon: class { fit() {} } }));
vi.mock("@xterm/addon-unicode11", () => ({ Unicode11Addon: class {} }));
vi.mock("@xterm/addon-search", () => ({ SearchAddon: class { clearDecorations() {} findNext() {} findPrevious() {} } }));
vi.mock("@xterm/addon-webgl", () => ({ WebglAddon: class { onContextLoss() {} dispose() {} } }));
vi.mock("@xterm/addon-web-links", () => ({ WebLinksAddon: class {} }));
vi.mock("./api", () => ({ fsStat: vi.fn(), rememberTermSize: vi.fn(), resizeSession: vi.fn(async () => undefined), writeSession: vi.fn(async () => undefined) }));
vi.mock("./store", () => ({ openFileAt: vi.fn(), selectedWorktree: () => "", showNotice: vi.fn() }));
vi.mock("./Confirm", () => ({ ask: vi.fn(async () => true) }));

import { TerminalView } from "./TerminalView";
import { onTermDrop } from "./terminal/dragDrop";

const flush = async () => {
  for (let i = 0; i < 5; i++) await Promise.resolve();
};

beforeEach(() => {
  h.terms.length = 0;
  h.dragListeners = 0;
  h.dragUnlistens = 0;
  h.invoke.mockClear();
  globalThis.ResizeObserver ??= class {
    observe() {}
    disconnect() {}
    unobserve() {}
  } as unknown as typeof ResizeObserver;
  globalThis.requestAnimationFrame ??= ((cb: () => void) => setTimeout(cb, 0)) as never;
  window.matchMedia ??= (() => ({ addEventListener() {}, removeEventListener() {} })) as never;
});
afterEach(() => cleanup());

describe("TerminalView lifecycle", () => {
  it("attaches on mount and detaches + disposes xterm on unmount", async () => {
    const r = render(() => createComponent(TerminalView, { id: 7 }));
    await flush();
    expect(h.invoke).toHaveBeenCalledWith("session_attach", expect.objectContaining({ id: 7 }));
    expect(h.terms).toHaveLength(1);
    r.unmount();
    expect(h.terms[0].disposed).toBe(true);
    expect(h.invoke).toHaveBeenCalledWith("session_detach", expect.objectContaining({ id: 7 }));
  });

  it("many terminals share one drag-drop listener, released with the last one", async () => {
    const a = render(() => createComponent(TerminalView, { id: 1 }));
    const b = render(() => createComponent(TerminalView, { id: 2 }));
    await flush();
    expect(h.dragListeners).toBe(1);
    a.unmount();
    expect(h.dragUnlistens).toBe(0);
    b.unmount();
    expect(h.dragUnlistens).toBe(1);
    // mounting again registers again
    const c = render(() => createComponent(TerminalView, { id: 3 }));
    await flush();
    expect(h.dragListeners).toBe(2);
    c.unmount();
    expect(h.dragUnlistens).toBe(2);
  });

  it("a mount/unmount churn leaves no listener behind", async () => {
    for (let i = 0; i < 5; i++) render(() => createComponent(TerminalView, { id: i })).unmount();
    await flush();
    expect(h.dragUnlistens).toBe(h.dragListeners);
  });
});

describe("onTermDrop", () => {
  it("delivers drop events only, to every subscriber, until unsubscribed", async () => {
    const got: string[] = [];
    const off = onTermDrop((p) => got.push(p.paths[0]));
    await flush();
    h.dragHandler?.({ payload: { type: "over", position: { x: 0, y: 0 } } });
    h.dragHandler?.({ payload: { type: "drop", paths: ["a"], position: { x: 0, y: 0 } } });
    expect(got).toEqual(["a"]);
    off();
    expect(h.dragUnlistens).toBe(1);
  });
});
