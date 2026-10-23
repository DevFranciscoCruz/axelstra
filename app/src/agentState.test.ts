import { describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: async () => undefined }));
vi.mock("./api", () => ({ call: async () => ({}), callBg: async () => ({}), listSessions: async () => [], listProjects: async () => [], worktreeStatus: async () => [], ghListBg: async () => [], gitChanges: async () => null, flowTasks: async () => [] }));

import type { SessionInfo } from "./api";
import { DOT_GLYPH, DOT_LABEL, dotOfSession, dotOfWorktree } from "./agentState";

const sess = (over: Partial<SessionInfo>): SessionInfo => ({ id: 1, running: true, meta: { kind: "agent" }, ...over }) as SessionInfo;

describe("status dots", () => {
  it("every visible state has its own shape and its own words", () => {
    const states = ["working", "idle", "waiting", "done", "attention", "queued", "running"] as const;
    expect(new Set(states.map((s) => DOT_GLYPH[s])).size).toBe(states.length);
    expect(new Set(states.map((s) => DOT_LABEL[s])).size).toBe(states.length);
    expect(DOT_LABEL.none).toBe("");
  });
  it("tells agent sessions apart", () => {
    expect(dotOfSession(sess({ idle_ms: 100 }))).toBe("working");
    expect(dotOfSession(sess({ idle_ms: 60000 }))).toBe("idle");
    expect(dotOfSession(sess({ waiting: true }))).toBe("waiting");
    expect(dotOfSession(sess({ running: false }))).toBe("done");
  });
  it("a plain terminal only says whether it runs", () => {
    expect(dotOfSession(sess({ meta: { kind: "shell" } }))).toBe("running");
    expect(dotOfSession(sess({ meta: { kind: "shell" }, running: false }))).toBe("done");
  });
  it("a worktree shows the most urgent thing its agents do", () => {
    const base = { queued: false, waiting: false, working: false, needs: false, live: 0 };
    expect(dotOfWorktree(base)).toBe("none");
    expect(dotOfWorktree({ ...base, live: 2 })).toBe("idle");
    expect(dotOfWorktree({ ...base, live: 2, needs: true })).toBe("attention");
    expect(dotOfWorktree({ ...base, live: 2, working: true, needs: true })).toBe("working");
    expect(dotOfWorktree({ ...base, live: 2, working: true, waiting: true })).toBe("waiting");
    expect(dotOfWorktree({ ...base, queued: true })).toBe("queued");
  });
});
