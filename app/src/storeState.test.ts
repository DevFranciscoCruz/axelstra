import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Project, SessionInfo } from "./api";

const invoke = vi.hoisted(() => vi.fn(async (_cmd: string, _args?: unknown) => undefined));
const daemon = vi.hoisted(() => ({
  sessions: [] as unknown[],
  projects: [] as unknown[],
  overview: 0,
  ghList: 0,
  failSessions: null as unknown,
  failRename: false,
  subs: new Map<string, Set<() => void>>(),
}));
const emit = (name: string) => daemon.subs.get(name)?.forEach((f) => f());

vi.mock("@tauri-apps/api/core", () => ({ invoke }));
vi.mock("./Confirm", () => ({ ask: vi.fn(async () => true) }));
vi.mock("./api", () => ({
  onDaemonEvent: (name: string, fn: () => void) => {
    let set = daemon.subs.get(name);
    if (!set) daemon.subs.set(name, (set = new Set()));
    set.add(fn);
    return () => void set!.delete(fn);
  },
  listSessions: async () => {
    if (daemon.failSessions) throw daemon.failSessions;
    return daemon.sessions;
  },
  listProjects: async () => daemon.projects,
  worktreeStatus: async () => [],
  worktreeOverview: async () => {
    daemon.overview++;
    return [];
  },
  ghListBg: async () => {
    daemon.ghList++;
    return [];
  },
  prRefresh: async () => ({}),
  gitChanges: async () => null,
  flowTasks: async () => [],
  callBg: async () => ({}),
  call: async (method: string) => {
    if (method === "session.rename" && daemon.failRename) throw new Error("disk full");
    return {};
  },
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
const load = async () => {
  vi.resetModules();
  return await import("./store");
};
const sess = (id: number, worktree: string, over: Partial<SessionInfo> = {}): SessionInfo => ({
  id,
  running: true,
  idle_ms: 9000,
  label: null,
  meta: { kind: "shell", worktree },
  ...over,
});

beforeEach(() => {
  vi.stubGlobal("localStorage", memory());
  daemon.sessions = [];
  daemon.projects = [];
  daemon.overview = 0;
  daemon.ghList = 0;
  daemon.subs.clear();
  daemon.failSessions = null;
  daemon.failRename = false;
});
afterEach(() => vi.useRealTimers());

describe("one view signal (F2)", () => {
  it("opening a file with Docker open shows the file, and vice versa", async () => {
    const s = await load();
    s.setShowDocker(true);
    expect(s.view().kind).toBe("docker");
    s.setOpenFile("src/a.ts");
    expect(s.view()).toEqual({ kind: "file", path: "src/a.ts", line: null });
    expect(s.showDocker()).toBe(false);
    s.setShowDocker(true);
    expect(s.openFile()).toBeNull();
  });

  it("closing a view only closes it while it is the one showing", async () => {
    const s = await load();
    s.setOpenDiff("a.ts");
    s.setOpenFile(null);
    expect(s.openDiff()).toBe("a.ts");
    s.setOpenDiff(null);
    expect(s.view().kind).toBe("terminal");
  });

  it("showView and openFileAt set the one view; the line goes with the file", async () => {
    const s = await load();
    s.openFileAt("x.ts", 12);
    expect(s.view()).toEqual({ kind: "file", path: "x.ts", line: 12 });
    expect(s.fileLine()).toBe(12);
    s.showView({ kanban: true });
    expect(s.showKanban()).toBe(true);
    expect(s.fileLine()).toBeNull();
    s.showView({ docker: true });
    expect(s.showKanban()).toBe(false);
    expect(s.showDocker()).toBe(true);
    s.showView();
    expect(s.view().kind).toBe("terminal");
  });
});

describe("sessions and projects keep identity when nothing changed (F4)", () => {
  it("keepSame returns the previous array for an identical list", async () => {
    const s = await load();
    const prev = [
      { id: 1, v: 1 },
      { id: 2, v: 2 },
    ];
    expect(
      s.keepSame(
        prev,
        [
          { id: 1, v: 1 },
          { id: 2, v: 2 },
        ],
        (x) => x.id,
      ),
    ).toBe(prev);
    const changed = s.keepSame(
      prev,
      [
        { id: 1, v: 1 },
        { id: 2, v: 3 },
      ],
      (x) => x.id,
    );
    expect(changed).not.toBe(prev);
    expect(changed[0]).toBe(prev[0]);
    expect(s.keepSame(prev, [{ id: 1, v: 1 }], (x) => x.id)).not.toBe(prev);
  });

  it("a poll that finds the same sessions leaves the signal's array alone", async () => {
    const s = await load();
    daemon.sessions = [sess(1, "C:/w/a")];
    await s.refreshSessions();
    const first = s.sessions();
    daemon.sessions = [sess(1, "C:/w/a")];
    await s.refreshSessions();
    expect(s.sessions()).toBe(first);
  });

  it("sessionsOf finds a worktree's sessions whatever the slashes and case, from one index", async () => {
    const s = await load();
    daemon.sessions = [sess(1, "C:\\W\\A"), sess(2, "c:/w/b"), sess(3, "C:/w/a/")];
    await s.refreshSessions();
    expect(s.sessionsOf("c:/w/a").map((x) => x.id)).toEqual([1, 3]);
    expect(s.sessionsOf("C:\\w\\b\\").map((x) => x.id)).toEqual([2]);
    expect(s.sessionsOf(null)).toEqual([]);
    expect(s.sessionsOf("C:/nowhere")).toEqual([]);
    expect(s.sessionsOf("C:/w/a")).toBe(s.sessionsOf("C:/w/a"));
  });
});

describe("failures are a list by key (F19/U5)", () => {
  it("two different failures stack; a successful poll clears only its own", async () => {
    const s = await load();
    s.setError("rename failed");
    daemon.failSessions = new Error("pipe closed");
    await s.refreshSessions();
    expect(s.toasts().filter((t) => t.kind === "error")).toHaveLength(2);
    daemon.failSessions = null;
    await s.refreshSessions();
    const left = s.toasts().filter((t) => t.kind === "error");
    expect(left).toHaveLength(1);
    expect(left[0].text).toBe("rename failed");
    s.setError("");
    expect(s.error()).toBe("");
  });

  it("the same failure twice shows once", async () => {
    const s = await load();
    s.setError("boom");
    s.setError("boom");
    expect(s.toasts()).toHaveLength(1);
  });

  it("a failed rename is reported, not swallowed", async () => {
    const s = await load();
    daemon.failRename = true;
    s.renameTab(1, "x");
    await new Promise((r) => setTimeout(r, 0));
    expect(s.error()).toContain("disk full");
  });

  it("an error is also recorded in the notification center; a poll failure is not", async () => {
    const s = await load();
    const n = await import("./notifications");
    s.setError("boom happened");
    expect(n.notifications().some((e) => e.kind === "error" && e.text.includes("boom happened"))).toBe(true);
    const before = n.notifications().length;
    daemon.failSessions = new Error("poll down");
    await s.refreshSessions();
    expect(n.notifications()).toHaveLength(before);
  });

  it("showNotice and toast({action}) are exported for other components", async () => {
    const s = await load();
    const run = vi.fn();
    s.showNotice("feito");
    s.toast({ text: "Apagado", action: { label: "Desfazer", run } });
    const t = s.toasts().find((x) => x.text === "Apagado")!;
    expect(t.action!.label).toBe("Desfazer");
    expect(s.notice()).toBe("Apagado");
    t.action!.run();
    expect(run).toHaveBeenCalled();
  });
});

describe("persisted JSON is validated on load (F10)", () => {
  it("pins that are not an array of strings load as empty", async () => {
    localStorage.setItem("axelstra.pins", '{"a":1}');
    expect((await load()).isPinned("a")).toBe(false);
    localStorage.setItem("axelstra.pins", "[1,2]");
    expect((await load()).isPinned("1")).toBe(false);
    localStorage.setItem("axelstra.pins", '["c:/x"]');
    expect((await load()).isPinned("C:\\X")).toBe(true);
  });

  it("project prefs keep only well-formed entries", async () => {
    const s = await load();
    expect(s.parsePrefs({ a: { agent: "codex", terse: "tiny" }, b: 3, c: { terse: "loud" }, d: [] })).toEqual({
      a: { agent: "codex", terse: "tiny" },
    });
    expect(s.parsePrefs(null)).toEqual({});
  });
});

describe("shared overview refresh (event driven)", () => {
  it("one subscription for any number of watchers, a refresh per event, no timer, and it ends with the last", async () => {
    vi.useFakeTimers();
    const s = await load();
    const stopA = s.watchOverview();
    const stopB = s.watchOverview();
    await vi.advanceTimersByTimeAsync(0);
    expect(daemon.overview).toBe(1);
    expect(vi.getTimerCount()).toBe(0);
    await vi.advanceTimersByTimeAsync(60_000);
    expect(daemon.overview).toBe(1);
    for (const ev of ["projects.changed", "session.changed", "dispatch.changed", "janitor.events", "pr.changed"]) {
      const before = daemon.overview;
      emit(ev);
      await vi.advanceTimersByTimeAsync(0);
      expect(daemon.overview, ev).toBe(before + 1);
    }
    stopA();
    const mid = daemon.overview;
    emit("projects.changed");
    await vi.advanceTimersByTimeAsync(0);
    expect(daemon.overview).toBe(mid + 1);
    stopB();
    emit("projects.changed");
    await vi.advanceTimersByTimeAsync(60_000);
    expect(daemon.overview).toBe(mid + 1);
  });
});

describe("pr.changed (event driven)", () => {
  it("reads the PR badges again at once, past their 3 minute limit, and stops with the last watcher", async () => {
    vi.useFakeTimers();
    const s = await load();
    s.setProjects([{ id: "p", name: "p", path: "C:/p", remote: true, worktrees: [] } as unknown as Project]);
    const stop = s.watchOverview();
    await vi.advanceTimersByTimeAsync(0);
    expect(daemon.ghList).toBe(0);
    emit("pr.changed");
    await vi.advanceTimersByTimeAsync(0);
    expect(daemon.ghList).toBe(1);
    emit("pr.changed");
    await vi.advanceTimersByTimeAsync(0);
    expect(daemon.ghList).toBe(2);
    expect(vi.getTimerCount()).toBe(0);
    stop();
    emit("pr.changed");
    await vi.advanceTimersByTimeAsync(0);
    expect(daemon.ghList).toBe(2);
  });
});

describe("projects", () => {
  it("api sees the projects through the context, not by importing the store", async () => {
    const s = await load();
    const { apiContext } = await import("./apiContext");
    const p = { id: "p", name: "p", path: "C:/p", worktrees: [] } as Project;
    s.setProjects([p]);
    expect(apiContext.projects()).toEqual([p]);
    s.setProjectPref("p", { terse: "tiny" });
    expect(apiContext.terse("p")).toBe("tiny");
  });
});
