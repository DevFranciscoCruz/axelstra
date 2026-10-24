import { beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({
  answer: true,
  asked: [] as { message: string; opts: Record<string, unknown> }[],
  status: undefined as undefined | { dirty?: number; ahead: number; upstream?: boolean },
  sessions: [] as { id: number; running: boolean }[],
  calls: [] as string[],
  notices: [] as string[],
}));

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("../Confirm", () => ({
  ask: vi.fn(async (message: string, opts: Record<string, unknown>) => {
    h.asked.push({ message, opts });
    return h.answer;
  }),
}));
vi.mock("../api", () => ({
  killSession: vi.fn(async (id: number) => void h.calls.push(`kill ${id}`)),
  removeSession: vi.fn(async (id: number) => void h.calls.push(`rmsession ${id}`)),
  removeWorktree: vi.fn(async (_p: string, path: string) => void h.calls.push(`rmwt ${path}`)),
  removeProject: vi.fn(),
  worktreeSweep: vi.fn(async () => []),
}));
vi.mock("../store", () => ({
  norm: (p?: string | null) => (p ?? "").toLowerCase(),
  refreshProjects: vi.fn(async () => undefined),
  refreshSessions: vi.fn(async () => undefined),
  selectWorktree: vi.fn(),
  selectedWorktree: () => null,
  sessionsOf: () => h.sessions,
  setError: vi.fn(),
  showNotice: (m: string) => void h.notices.push(m),
  wtStatus: () => ({ "/w/x": h.status }),
}));

import { createSidebarActions } from "./actions";

beforeEach(() => {
  h.answer = true;
  h.asked.length = 0;
  h.status = undefined;
  h.sessions = [];
  h.calls.length = 0;
  h.notices.length = 0;
});

describe("remove worktree flow", () => {
  it("unsaved work asks for the typed name and lists what is lost", async () => {
    h.status = { dirty: 2, ahead: 1, upstream: true };
    await createSidebarActions().dropWorktree("p", "/w/x", "feat-x");
    expect(h.asked[0].opts).toMatchObject({ kind: "danger", typeToConfirm: "feat-x" });
    expect(h.asked[0].message).toContain("Vais perder");
    expect(h.calls).toEqual(["rmwt /w/x"]);
  });

  it("a clean worktree needs no typed name", async () => {
    h.status = { dirty: 0, ahead: 0 };
    await createSidebarActions().dropWorktree("p", "/w/x", "feat-x");
    expect(h.asked[0].opts.typeToConfirm).toBeUndefined();
  });

  it("declining removes nothing", async () => {
    h.answer = false;
    h.status = { dirty: 1, ahead: 0 };
    h.sessions = [{ id: 7, running: true }];
    await createSidebarActions().dropWorktree("p", "/w/x", "feat-x");
    expect(h.calls).toEqual([]);
  });

  it("running sessions are killed and removed before the worktree", async () => {
    h.sessions = [
      { id: 7, running: true },
      { id: 8, running: false },
    ];
    await createSidebarActions().dropWorktree("p", "/w/x", "feat-x");
    expect(h.calls).toEqual(["kill 7", "rmsession 7", "rmsession 8", "rmwt /w/x"]);
    expect(h.notices.at(-1)).toContain("feat-x");
  });
});
