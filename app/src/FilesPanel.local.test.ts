import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, render } from "@solidjs/testing-library";
import { createComponent } from "solid-js";

// Real api.ts on top of a fake host: `daemon_call` never answers (a stuck daemon), the local commands do.
const h = vi.hoisted(() => ({
  wt: "C:/wt",
  calls: [] as { cmd: string; args: Record<string, unknown> }[],
  channels: [] as { onmessage: (c: unknown) => void }[],
  local: undefined as undefined | (() => Promise<unknown>),
  listeners: new Map<string, (e: { payload: unknown }) => void>(),
}));

vi.mock("@tauri-apps/api/core", () => {
  class Channel {
    onmessage: (c: unknown) => void = () => {};
    constructor() {
      h.channels.push(this);
    }
  }
  return {
    Channel,
    invoke: vi.fn((cmd: string, args: Record<string, unknown> = {}) => {
      h.calls.push({ cmd, args });
      if (cmd === "local_fs_list") return h.local!();
      if (cmd === "daemon_call" || cmd === "daemon_call_bg") return new Promise(() => {});
      return Promise.resolve(undefined);
    }),
  };
});
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (ev: string, fn: (e: { payload: unknown }) => void) => {
    h.listeners.set(ev, fn);
    return () => h.listeners.delete(ev);
  }),
}));
vi.mock("./store", () => ({
  changes: () => [],
  findWorktree: () => ({ project: { name: "proj" }, wt: { path: h.wt, branch: "main" } }),
  fsTick: () => 0,
  guarded: (f: () => void) => f(),
  openFile: () => null,
  selectedWorktree: () => h.wt,
  setError: vi.fn(),
  setOpenCommit: vi.fn(),
  setOpenDiff: vi.fn(),
  setOpenFile: vi.fn(),
  setShowKanban: vi.fn(),
  rightTab: () => "files",
  showNotice: vi.fn(),
}));
vi.mock("./RightTabs", () => ({ RightTabs: () => null }));
vi.mock("./Confirm", () => ({ ask: vi.fn(async () => true) }));
vi.mock("./ContextMenu", () => ({ menuKey: () => false, showMenu: vi.fn() }));

import { FilesPanel } from "./FilesPanel";
import { fsListStream } from "./api";

const settle = async () => {
  for (let i = 0; i < 20; i++) await Promise.resolve();
};
const ents = [
  { name: "src", dir: true },
  { name: "a.txt", dir: false },
];
const cmds = (c: string) => h.calls.filter((x) => x.cmd === c);

beforeEach(() => {
  h.wt = "C:/wt";
  h.calls.length = 0;
  h.channels.length = 0;
  h.listeners.clear();
  h.local = async () => ({ entries: ents, total: 2, truncated: false });
});
afterEach(() => cleanup());

describe("explorer without the daemon", () => {
  it("lists a local folder while every daemon call hangs", async () => {
    const r = render(() => createComponent(FilesPanel, {}));
    await settle();
    expect(r.container.textContent).toContain("a.txt");
    expect(cmds("local_fs_list")[0].args).toMatchObject({ worktree: "C:/wt", path: "" });
    expect(cmds("daemon_call").length).toBe(0);
    expect(r.container.textContent).not.toContain("A carregar");
  });

  it("draws chunks as they arrive with a loading row, then the sorted answer", async () => {
    let finish!: (v: unknown) => void;
    h.local = () => new Promise((res) => (finish = res));
    const r = render(() => createComponent(FilesPanel, {}));
    await settle();
    h.channels[0].onmessage([
      { name: "zeta.txt", dir: false },
      { name: "lib", dir: true },
    ]);
    await settle();
    const text = r.container.textContent!;
    expect(text).toContain("zeta.txt");
    expect(text.indexOf("lib")).toBeLessThan(text.indexOf("zeta.txt"));
    const busy = r.container.querySelector(".floading");
    expect(busy?.getAttribute("role")).toBe("status");
    expect(busy?.getAttribute("aria-busy")).toBe("true");
    finish({ entries: ents, total: 2, truncated: false });
    await settle();
    expect(r.container.querySelector(".floading")).toBeNull();
    expect(r.container.textContent).toContain("a.txt");
    expect(r.container.textContent).not.toContain("zeta.txt");
  });

  it("the stop button cancels the host listing and shows the coded error", async () => {
    let fail!: (e: unknown) => void;
    h.local = () => new Promise((_, rej) => (fail = rej));
    const r = render(() => createComponent(FilesPanel, {}));
    await settle();
    h.channels[0].onmessage([{ name: "x", dir: false }]);
    await settle();
    const id = cmds("local_fs_list")[0].args.req;
    (r.container.querySelector(".floading button") as HTMLButtonElement).click();
    expect(cmds("local_fs_cancel")[0].args).toEqual({ req: id });
    fail({ message: "cancelled", code: "localfs.cancelled" });
    await settle();
    expect(r.container.querySelector(".ferr")?.textContent).toContain("Cancelado");
  });

  it("a truncated folder says so", async () => {
    h.local = async () => ({ entries: ents, total: 20000, truncated: true });
    const r = render(() => createComponent(FilesPanel, {}));
    await settle();
    expect(r.container.querySelector(".ftrunc")?.textContent).toMatch(/20\D?000/);
  });

  it("a local error (outside roots) is final: shown, no daemon retry", async () => {
    h.local = async () => Promise.reject({ message: "outside", code: "localfs.outside_roots", params: { path: "C:/wt" } });
    const r = render(() => createComponent(FilesPanel, {}));
    await settle();
    expect(r.container.querySelector(".ferr")?.getAttribute("role")).toBe("alert");
    expect(r.container.querySelector(".ferr")?.textContent).toContain("Fora dos projetos");
    expect(cmds("daemon_call").length).toBe(0);
  });

  it("localfs.not_local falls back to the daemon", async () => {
    h.local = async () => Promise.reject({ message: "x", code: "localfs.not_local" });
    render(() => createComponent(FilesPanel, {}));
    await settle();
    const d = cmds("daemon_call");
    expect(d.length).toBe(1);
    expect(d[0].args).toMatchObject({ method: "fs.list" });
  });

  it("a remote worktree goes straight to the daemon and shows the loading state, no error", async () => {
    h.wt = "@lab:/home/me/proj";
    const r = render(() => createComponent(FilesPanel, {}));
    await settle();
    expect(cmds("local_fs_list").length).toBe(0);
    expect(cmds("daemon_call")[0].args).toMatchObject({ method: "fs.list" });
    expect(r.container.querySelector("[aria-busy=true]")).not.toBeNull();
    expect(r.container.querySelector(".ferr")).toBeNull();
  });

  it("leaving the panel cancels the running listing", async () => {
    h.local = () => new Promise(() => {});
    const r = render(() => createComponent(FilesPanel, {}));
    await settle();
    const id = cmds("local_fs_list")[0].args.req;
    r.unmount();
    expect(cmds("local_fs_cancel").some((c) => c.args.req === id)).toBe(true);
  });

  it("each listing carries its own request id", () => {
    h.local = () => new Promise(() => {});
    void fsListStream("C:/wt", "", 1, () => {});
    void fsListStream("C:/wt", "", 2, () => {});
    expect(cmds("local_fs_list").map((c) => c.args.req)).toEqual([1, 2]);
  });

  it("starts the host watcher and refreshes on local-fs-changed", async () => {
    render(() => createComponent(FilesPanel, {}));
    await settle();
    expect(cmds("local_fs_watch")[0].args).toEqual({ worktree: "C:/wt" });
    vi.useFakeTimers();
    h.listeners.get("local-fs-changed")!({ payload: { worktree: "C:/wt" } });
    await vi.advanceTimersByTimeAsync(300);
    vi.useRealTimers();
    await settle();
    expect(cmds("local_fs_list").length).toBe(2);
  });
});
