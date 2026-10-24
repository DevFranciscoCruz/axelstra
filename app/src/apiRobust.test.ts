/* eslint-disable @typescript-eslint/no-explicit-any -- loose test doubles for the arguments of the mocked invoke */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

type Call = { method: string; params: Record<string, any> };
const calls: Call[] = [];
const writes: { id: number; data: string }[] = [];
const state = vi.hoisted(() => ({
  pending: [] as any[],
  hang: false,
  fail: null as unknown,
  sessions: [] as any[],
  watchStarts: 0,
  channels: [] as any[],
  listeners: new Map<string, Array<() => void>>(),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (ev: string, cb: () => void) => {
    const l = state.listeners.get(ev) ?? [];
    l.push(cb);
    state.listeners.set(ev, l);
    return () =>
      state.listeners.set(
        ev,
        (state.listeners.get(ev) ?? []).filter((x) => x !== cb),
      );
  }),
}));
vi.mock("@tauri-apps/api/core", () => ({
  Channel: class {
    onmessage: (m: unknown) => void = () => {};
    constructor() {
      state.channels.push(this);
    }
  },
  invoke: vi.fn(async (cmd: string, args: any) => {
    if (cmd === "default_shell") return ["pwsh.exe", ["-NoLogo"]];
    if (cmd === "watch_start") {
      state.watchStarts++;
      return null;
    }
    if (cmd === "session_write") {
      writes.push({ id: args.id, data: args.data });
      return null;
    }
    if (cmd !== "daemon_call" && cmd !== "daemon_call_bg") return null;
    if (state.hang) return new Promise(() => {});
    if (state.fail !== null) throw state.fail;
    calls.push({ method: args.method, params: args.params });
    switch (args.method) {
      case "session.pending":
        return [...state.pending];
      case "session.pending_remove":
        state.pending = state.pending.filter(
          (p) => p !== args.params.rec && JSON.stringify(p) !== JSON.stringify(args.params.rec),
        );
        return {};
      case "session.list":
        return state.sessions;
      case "settings.get":
        return { rules: null };
      case "app.dir":
        return { dir: "C:/data" };
      case "session.spawn":
        return { id: calls.length };
      default:
        return {};
    }
  }),
}));

import { AppError } from "./appError";
import {
  call,
  cancelNudge,
  nudge,
  restoreSessions,
  stopWatch,
  watchWorktree,
  writeSession,
  CALL_TIMEOUT_MS,
} from "./api";

beforeEach(() => {
  calls.length = 0;
  writes.length = 0;
  state.pending = [];
  state.hang = false;
  state.fail = null;
  state.sessions = [];
  state.watchStarts = 0;
  state.channels.length = 0;
  state.listeners.clear();
});
afterEach(() => {
  vi.useRealTimers();
  stopWatch();
});

describe("call", () => {
  it("fails with an AppError naming the method when the daemon never answers", async () => {
    vi.useFakeTimers();
    state.hang = true;
    const p = call("session.list");
    const seen = p.catch((e: any) => e);
    await vi.advanceTimersByTimeAsync(CALL_TIMEOUT_MS + 10);
    const e = (await seen) as AppError;
    expect(e).toBeInstanceOf(AppError);
    expect(e.method).toBe("session.list");
    expect(String(e)).toContain("Sem resposta do daemon");
  });

  it("gives slow methods (git, docker) longer than the default", async () => {
    vi.useFakeTimers();
    state.hang = true;
    const p = call("git.push", {}).catch((e) => e);
    await vi.advanceTimersByTimeAsync(CALL_TIMEOUT_MS + 10);
    let done = false;
    void p.then(() => (done = true));
    await vi.advanceTimersByTimeAsync(0);
    expect(done).toBe(false);
    await vi.advanceTimersByTimeAsync(5 * 60_000);
    expect(await p).toBeInstanceOf(AppError);
  });

  it("turns an object error into readable text, never [object Object]", async () => {
    state.fail = { message: "rejected by remote" };
    await expect(call("git.push")).rejects.toThrow("rejected by remote");
    state.fail = { code: 7 };
    const e = await call("x.y").catch((x) => x);
    expect(String(e)).toBe('{"code":7}');
    state.fail = {};
    expect(String(await call("x.y").catch((x) => x))).toBe("Erro desconhecido");
  });
});

describe("restoreSessions single flight", () => {
  const entries = () => [
    { program: "pwsh.exe", label: "a", cwd: "C:/w" },
    { program: "pwsh.exe", label: "b", cwd: "C:/w" },
    { program: "pwsh.exe", label: "c", cwd: "C:/w" },
  ];

  it("two overlapping restores (mount + daemon-restarted) start every session once", async () => {
    state.pending = entries();
    const [r1, r2] = await Promise.all([restoreSessions(), restoreSessions()]);
    expect(calls.filter((c) => c.method === "session.spawn")).toHaveLength(3);
    expect(r1.restored + r2.restored).toBe(3);
  });

  it("a later restore (new pending) still runs", async () => {
    state.pending = entries().slice(0, 1);
    await restoreSessions();
    state.pending = entries().slice(1);
    const r = await restoreSessions();
    expect(r.restored).toBe(2);
    expect(calls.filter((c) => c.method === "session.spawn")).toHaveLength(3);
  });

  it("a failed restore does not wedge the next one", async () => {
    state.fail = new Error("down");
    await expect(restoreSessions()).rejects.toThrow("down");
    state.fail = null;
    state.pending = entries();
    expect((await restoreSessions()).restored).toBe(3);
  });
});

describe("watchWorktree lifecycle", () => {
  it("registers each listener once however often it is called, and the newest handlers win", async () => {
    const a = vi.fn();
    const b = vi.fn();
    await watchWorktree("C:/a", a);
    await watchWorktree("C:/b", b);
    expect(state.listeners.get("watch-ended")).toHaveLength(1);
    expect(state.listeners.get("daemon-restarted")).toHaveLength(1);
    expect(state.watchStarts).toBe(1);
    state.channels[0].onmessage({ event: "fs.changed", worktree: "C:/b" });
    expect(a).not.toHaveBeenCalled();
    expect(b).toHaveBeenCalledWith("C:/b");
  });

  it("a restart opens a new channel and the old one goes quiet; two restart events open one", async () => {
    const h = vi.fn();
    await watchWorktree("C:/a", h);
    const old = state.channels[0];
    state.listeners.get("daemon-restarted")![0]();
    state.listeners.get("watch-ended")![0]();
    await new Promise((r) => setTimeout(r, 10));
    expect(state.watchStarts).toBe(2);
    expect(state.channels).toHaveLength(2);
    old.onmessage({ event: "fs.changed", worktree: "C:/a" });
    expect(h).not.toHaveBeenCalled();
    state.channels[1].onmessage({ event: "fs.changed", worktree: "C:/a" });
    expect(h).toHaveBeenCalledTimes(1);
  });

  it("stopWatch removes the listeners", async () => {
    await watchWorktree("C:/a", vi.fn());
    stopWatch();
    await new Promise((r) => setTimeout(r, 0));
    expect(state.listeners.get("watch-ended")).toHaveLength(0);
    expect(state.listeners.get("daemon-restarted")).toHaveLength(0);
  });
});

describe("nudge", () => {
  const idle = () => (state.sessions = [{ id: 7, running: true, idle_ms: 5000, label: null, meta: null }]);

  it("types the note once the screen has settled", async () => {
    vi.useFakeTimers();
    idle();
    const p = nudge(7, "note");
    await vi.advanceTimersByTimeAsync(4000 + 400 + 50);
    await p;
    expect(writes.map((w) => w.data)).toEqual(["note", "\r"]);
  });

  it("writes nothing when the user typed in the session first", async () => {
    vi.useFakeTimers();
    idle();
    const p = nudge(7, "note");
    await vi.advanceTimersByTimeAsync(1000);
    await writeSession(7, "hello");
    await vi.advanceTimersByTimeAsync(10_000);
    await p;
    expect(writes.map((w) => w.data)).toEqual(["hello"]);
  });

  it("can be cancelled and gives up quietly when the session is gone", async () => {
    vi.useFakeTimers();
    idle();
    const p = nudge(7, "note");
    cancelNudge(7);
    await vi.advanceTimersByTimeAsync(10_000);
    await p;
    expect(writes).toHaveLength(0);
    state.sessions = [];
    const q = nudge(7, "note");
    await vi.advanceTimersByTimeAsync(10_000);
    await q;
    expect(writes).toHaveLength(0);
  });
});
