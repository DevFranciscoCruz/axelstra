import { beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({
  answer: true,
  asked: [] as { message: string; opts: Record<string, unknown> }[],
  status: undefined as undefined | { dirty?: number; ahead: number; upstream?: boolean },
  sessions: [] as { id: number; running: boolean }[],
  calls: [] as string[],
  notices: [] as string[],
  mode: "list" as "list" | "all" | null,
  inProject: false,
  /** what worktree.status answers when the store has nothing yet (null: it fails) */
  read: null as null | { path: string; dirty: number; ahead: number; upstream: boolean },
  rmFails: false,
}));

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("../Confirm", () => ({
  ask: vi.fn(async (message: string, opts: Record<string, unknown>) => {
    h.asked.push({ message, opts });
    return h.answer;
  }),
}));
vi.mock("../RemoveProject", () => ({ askRemoveProject: vi.fn(async () => h.mode) }));
vi.mock("../api", () => ({
  killSession: vi.fn(async (id: number) => void h.calls.push(`kill ${id}`)),
  removeSession: vi.fn(async (id: number) => void h.calls.push(`rmsession ${id}`)),
  removeWorktree: vi.fn(async (_p: string, path: string) => {
    if (h.rmFails) throw new Error("folder in use");
    h.calls.push(`rmwt ${path}`);
  }),
  worktreeStatus: vi.fn(async () => {
    if (!h.read) throw new Error("no status");
    return [h.read];
  }),
  removeProject: vi.fn(async (id: string, mode: string) => {
    h.calls.push(`rmproject ${id} ${mode}`);
    return { removed: true, sessions_closed: 2, worktrees_removed: 1, worktrees_pending: 0, branches_deleted: 1 };
  }),
  worktreeSweep: vi.fn(async () => []),
}));
vi.mock("../store", () => ({
  dismissExited: vi.fn(),
  norm: (p?: string | null) => (p ?? "").toLowerCase(),
  refreshProjects: vi.fn(async () => undefined),
  refreshSessions: vi.fn(async () => undefined),
  findWorktree: () => (h.inProject ? { project: { id: "p" }, wt: {} } : null),
  selectWorktree: vi.fn(),
  selectedWorktree: () => null,
  sessionsOf: () => h.sessions,
  setError: vi.fn(),
  showNotice: (m: string) => void h.notices.push(m),
  wtStatus: () => (h.status ? { "/w/x": h.status } : {}),
}));

import { createSidebarActions } from "./actions";

beforeEach(() => {
  h.answer = true;
  h.asked.length = 0;
  h.status = undefined;
  h.sessions = [];
  h.calls.length = 0;
  h.notices.length = 0;
  h.mode = "list";
  h.inProject = false;
  h.read = null;
  h.rmFails = false;
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

  it("reads the state on demand when the store has none yet", async () => {
    h.read = { path: "/w/x", dirty: 3, ahead: 0, upstream: true };
    await createSidebarActions().dropWorktree("p", "/w/x", "feat-x");
    expect(h.asked[0].opts).toMatchObject({ typeToConfirm: "feat-x" });
  });

  it("an unreadable state is treated as risky: typed name and a warning in the question", async () => {
    await createSidebarActions().dropWorktree("p", "/w/x", "feat-x");
    expect(h.asked[0].opts).toMatchObject({ kind: "danger", typeToConfirm: "feat-x" });
    expect(h.asked[0].message).toContain("Não foi possível ler o estado");
  });

  it("a removal that fails after the sessions closed says so and offers to retry", async () => {
    h.status = { dirty: 0, ahead: 0 };
    h.rmFails = true;
    h.sessions = [{ id: 7, running: true }];
    const t = await import("../toasts");
    await createSidebarActions().dropWorktree("p", "/w/x", "feat-x");
    const err = t.toasts().find((x) => x.kind === "error");
    expect(err?.text).toContain("feat-x");
    expect(err?.action?.label).toBe("Tentar de novo");
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

describe("remove project flow", () => {
  it("cancelling the dialog removes nothing", async () => {
    h.mode = null;
    await createSidebarActions().dropProject("p", "meu");
    expect(h.calls).toEqual([]);
    expect(h.notices).toEqual([]);
  });

  it("'list' sends mode list and says the project left the list", async () => {
    await createSidebarActions().dropProject("p", "meu");
    expect(h.calls).toEqual(["rmproject p list"]);
    expect(h.notices.at(-1)).toBe("Projeto tirado da lista: meu");
  });

  it("'all' sends mode all and reports the counts", async () => {
    h.mode = "all";
    await createSidebarActions().dropProject("p", "meu");
    expect(h.calls).toEqual(["rmproject p all"]);
    expect(h.notices.at(-1)).toContain("2 sessões fechadas");
  });
});
