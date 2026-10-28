import { beforeEach, describe, expect, it, vi } from "vitest";

const n = vi.hoisted(() => ({ list: 0, status: 0, overview: 0, changes: 0, release: [] as (() => void)[] }));
const slow = <T>(v: T) =>
  new Promise<T>((res) => {
    n.release.push(() => res(v));
  });

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn(async () => undefined) }));
vi.mock("./Confirm", () => ({ ask: vi.fn(async () => true) }));
vi.mock("./api", () => ({
  listSessions: async () => [],
  listProjects: () => {
    n.list++;
    return slow([
      { id: "p", path: "C:/p", git: true, remote: false, worktrees: [{ path: "C:/p", branch: "main", is_main: true }] },
    ]);
  },
  worktreeStatus: () => {
    n.status++;
    return slow([]);
  },
  worktreeOverview: () => {
    n.overview++;
    return slow([]);
  },
  gitChanges: () => {
    n.changes++;
    return slow(null);
  },
  ghListBg: async () => [],
  prRefresh: async () => undefined,
  onDaemonEvent: () => () => undefined,
  apiContext: { projects: () => [], terse: () => undefined },
  call: async () => ({}),
  callBg: async () => ({}),
}));

import { coalesced, lazyRefresh, singleFlight } from "./requestGate";
import { refreshOverview, refreshProjects, refreshChanges } from "./store/git";

const settle = async () => {
  for (let i = 0; i < 5; i++) {
    while (n.release.length) n.release.shift()!();
    await Promise.resolve();
    await new Promise((r) => setTimeout(r, 0));
  }
};

beforeEach(() => {
  n.list = n.status = n.overview = n.changes = 0;
  n.release.length = 0;
});

describe("request volume", () => {
  it("20 project refreshes in a burst make at most 2 project.list", async () => {
    for (let i = 0; i < 20; i++) void refreshProjects();
    await settle();
    expect(n.list).toBeLessThanOrEqual(2);
  });

  it("40 overview events make at most 2 worktree.overview", async () => {
    for (let i = 0; i < 40; i++) void refreshOverview();
    await settle();
    expect(n.overview).toBeLessThanOrEqual(2);
  });

  it("30 change events make at most 2 git.changes while nothing is selected is zero", async () => {
    for (let i = 0; i < 30; i++) void refreshChanges();
    await settle();
    expect(n.changes).toBe(0);
  });

  it("project.list does not trigger worktree.status more than once per burst", async () => {
    for (let i = 0; i < 10; i++) {
      void refreshProjects();
      await settle();
    }
    expect(n.status).toBeLessThanOrEqual(2);
  });
});

describe("requestGate", () => {
  it("coalesced: calls during a run share one rerun", async () => {
    let runs = 0;
    const g = coalesced(async () => ++runs);
    const all = [g(), g(), g(), g()];
    await Promise.all(all);
    expect(runs).toBe(2);
  });

  it("singleFlight: same key shares the promise", async () => {
    let runs = 0;
    const f = () => singleFlight("k", async () => ++runs);
    await Promise.all([f(), f(), f()]);
    expect(runs).toBe(1);
  });

  it("lazyRefresh: hidden asks load once on show", () => {
    let vis = false;
    let runs = 0;
    const l = lazyRefresh(
      () => runs++,
      () => vis,
    );
    l.ask();
    l.ask();
    expect(runs).toBe(0);
    vis = true;
    l.shown();
    l.shown();
    expect(runs).toBe(1);
  });
});
