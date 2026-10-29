import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, render } from "@solidjs/testing-library";
import { createComponent } from "solid-js";

const h = vi.hoisted(() => ({
  listCalls: [] as string[],
  entries: [
    { name: "src", dir: true },
    { name: "README.md", dir: false },
  ],
  failList: false,
}));

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn(async () => undefined) }));
vi.mock("./api", () => ({
  fsListStream: vi.fn(async (_wt: string, path: string) => {
    h.listCalls.push(path);
    if (h.failList) throw new Error("sem acesso");
    const entries = path === "" ? h.entries : [];
    return { entries, total: entries.length, truncated: false };
  }),
  fsListCancel: vi.fn(),
  nextListReq: (() => {
    let n = 0;
    return () => ++n;
  })(),
  fsAct: vi.fn(async () => undefined),
  fsWatchLocal: vi.fn(async () => undefined),
  onLocalFsChanged: vi.fn(() => () => {}),
  fsSearch: vi.fn(async () => []),
  fsGrep: vi.fn(async () => []),
  fsCreate: vi.fn(),
  fsDelete: vi.fn(),
  fsRename: vi.fn(),
}));
vi.mock("./store", () => ({
  changes: () => [],
  findWorktree: () => ({ project: { name: "proj" }, wt: { path: "C:/wt", branch: "main" } }),
  fsTick: () => 0,
  guarded: (f: () => void) => f(),
  openFile: () => null,
  selectedWorktree: () => "C:/wt",
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

const settle = async () => {
  for (let i = 0; i < 10; i++) await Promise.resolve();
};

beforeEach(() => {
  h.listCalls.length = 0;
  h.failList = false;
});
afterEach(() => cleanup());

describe("FilesPanel", () => {
  it("lists the root of the selected worktree", async () => {
    const r = render(() => createComponent(FilesPanel, {}));
    await settle();
    expect(h.listCalls[0]).toBe("");
    expect(r.container.textContent).toContain("README.md");
    expect(r.container.textContent).toContain("src");
  });

  it("a failing listing shows the error, not a blank panel", async () => {
    h.failList = true;
    const r = render(() => createComponent(FilesPanel, {}));
    await settle();
    expect(r.container.textContent).toContain("sem acesso");
  });

  it("unmounting stops reacting to visibility changes", async () => {
    const add = vi.spyOn(document, "addEventListener");
    const remove = vi.spyOn(document, "removeEventListener");
    const r = render(() => createComponent(FilesPanel, {}));
    await settle();
    r.unmount();
    const added = add.mock.calls.filter((c) => c[0] === "visibilitychange").length;
    const removed = remove.mock.calls.filter((c) => c[0] === "visibilitychange").length;
    expect(removed).toBe(added);
    add.mockRestore();
    remove.mockRestore();
  });
});
