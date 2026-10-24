import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { SessionInfo } from "./api";

vi.mock("@tauri-apps/api/core", () => ({ invoke: async () => undefined }));
vi.mock("./Confirm", () => ({ ask: vi.fn(async () => true) }));
vi.mock("./Voice", () => ({ toggleVoice: vi.fn(async () => {}) }));
vi.mock("./api", () => ({
  listSessions: async () => [],
  listProjects: async () => [],
  worktreeStatus: async () => [],
  worktreeOverview: async () => [],
  ghListBg: async () => [],
  gitChanges: async () => null,
  flowTasks: async () => [],
  callBg: async () => ({}),
  call: async () => ({}),
}));

const memory = () => {
  const m = new Map<string, string>();
  return {
    getItem: (k: string) => m.get(k) ?? null,
    setItem: (k: string, v: string) => void m.set(k, v),
    removeItem: (k: string) => void m.delete(k),
    clear: () => m.clear(),
  };
};

const sess = (id: number, over: Partial<SessionInfo> = {}): SessionInfo => ({
  id,
  running: true,
  idle_ms: 9000,
  label: null,
  meta: { kind: "shell", worktree: "/w" },
  ...over,
});

const setup = async () => {
  vi.resetModules();
  const store = await import("./store");
  const { createShortcutHandler } = await import("./useGlobalShortcuts");
  const voice = await import("./Voice");
  const tabs = [sess(1), sess(2), sess(3)];
  const showTab = vi.fn((s: SessionInfo) => store.setActiveSession(s.id));
  const reopenClosed = vi.fn();
  store.setActiveSession(1);
  const handle = createShortcutHandler({ tabs: () => tabs, showTab, reopenClosed });
  const press = (init: KeyboardEventInit, target?: Element) => {
    const e = new KeyboardEvent("keydown", { bubbles: true, cancelable: true, ...init });
    if (target) Object.defineProperty(e, "target", { value: target });
    handle(e);
    return e;
  };
  return { store, voice, showTab, reopenClosed, press };
};

beforeEach(() => vi.stubGlobal("localStorage", memory()));
afterEach(() => {
  document.body.innerHTML = "";
});

describe("global shortcuts", () => {
  it("Ctrl+Tab and Ctrl+Shift+Tab cycle the tabs, wrapping", async () => {
    const { press, showTab, store } = await setup();
    press({ key: "Tab", ctrlKey: true });
    expect(showTab).toHaveBeenLastCalledWith(expect.objectContaining({ id: 2 }));
    store.setActiveSession(1);
    const e = press({ key: "Tab", ctrlKey: true, shiftKey: true });
    expect(showTab).toHaveBeenLastCalledWith(expect.objectContaining({ id: 3 }));
    expect(e.defaultPrevented).toBe(true);
  });

  it("Ctrl+n jumps to tab n and ignores a missing one", async () => {
    const { press, showTab } = await setup();
    press({ key: "3", ctrlKey: true });
    expect(showTab).toHaveBeenLastCalledWith(expect.objectContaining({ id: 3 }));
    showTab.mockClear();
    const e = press({ key: "9", ctrlKey: true });
    expect(showTab).not.toHaveBeenCalled();
    expect(e.defaultPrevented).toBe(false);
  });

  it("Ctrl+B and Ctrl+Shift+B toggle the panels", async () => {
    const { press, store } = await setup();
    const left = store.showSidebar();
    const right = store.showRightPanel();
    press({ key: "b", ctrlKey: true });
    expect(store.showSidebar()).toBe(!left);
    expect(store.showRightPanel()).toBe(right);
    press({ key: "B", ctrlKey: true, shiftKey: true });
    expect(store.showRightPanel()).toBe(!right);
  });

  it("Ctrl+, opens Settings, Ctrl+Shift+T reopens, Ctrl+Shift+F opens the chat search", async () => {
    const { press, store, reopenClosed } = await setup();
    press({ key: ",", ctrlKey: true });
    expect(store.showSettings()).toBe(true);
    // with a dialog open the page shortcuts leave the keys alone
    press({ key: "T", ctrlKey: true, shiftKey: true });
    expect(reopenClosed).not.toHaveBeenCalled();
    store.setShowSettings(false);
    press({ key: "T", ctrlKey: true, shiftKey: true });
    expect(reopenClosed).toHaveBeenCalledTimes(1);
    press({ key: "F", ctrlKey: true, shiftKey: true });
    expect(store.showAgentSearch()).toBe(true);
  });

  it("Ctrl+Shift+M toggles voice even from a text field", async () => {
    const { press, voice } = await setup();
    document.body.innerHTML = "<input id=i>";
    press({ key: "M", ctrlKey: true, shiftKey: true }, document.getElementById("i")!);
    expect(voice.toggleVoice).toHaveBeenCalledTimes(1);
  });

  it("'?' opens the cheat sheet, but not while typing or over a dialog", async () => {
    const { press, store } = await setup();
    document.body.innerHTML = "<input id=i><div id=d></div>";
    press({ key: "?" }, document.getElementById("i")!);
    expect(store.showShortcuts()).toBe(false);
    document.body.insertAdjacentHTML("beforeend", '<div role="dialog"></div>');
    press({ key: "?" }, document.getElementById("d")!);
    expect(store.showShortcuts()).toBe(false);
    document.querySelector('[role="dialog"]')!.remove();
    press({ key: "?" }, document.getElementById("d")!);
    expect(store.showShortcuts()).toBe(true);
  });

  it("tab and panel shortcuts leave text fields and dialogs alone", async () => {
    const { press, showTab, store } = await setup();
    document.body.innerHTML = "<textarea id=t></textarea>";
    const left = store.showSidebar();
    press({ key: "Tab", ctrlKey: true }, document.getElementById("t")!);
    press({ key: "b", ctrlKey: true }, document.getElementById("t")!);
    expect(showTab).not.toHaveBeenCalled();
    expect(store.showSidebar()).toBe(left);
    document.body.innerHTML = '<div role="dialog"></div>';
    press({ key: "2", ctrlKey: true });
    expect(showTab).not.toHaveBeenCalled();
  });

  it("Ctrl+J with no agent waiting says so", async () => {
    const { press } = await setup();
    const { toasts } = await import("./store");
    press({ key: "j", ctrlKey: true });
    expect(toasts().some((t) => t.text.includes("à espera"))).toBe(true);
  });

  it("Alt-modified chords are not shortcuts", async () => {
    const { press, store } = await setup();
    press({ key: ",", ctrlKey: true, altKey: true });
    expect(store.showSettings()).toBe(false);
  });
});
