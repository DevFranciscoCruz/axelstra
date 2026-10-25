/* eslint-disable @typescript-eslint/no-explicit-any -- loose test doubles for the arguments of the mocked invoke */
import { beforeEach, describe, expect, it, vi } from "vitest";

const calls: { method: string; params: Record<string, any> }[] = [];
let pending: unknown[] = [];
let live: unknown[] = [];
let failSpawnFor: string | null = null;

vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(() => Promise.resolve(() => {})) }));
vi.mock("@tauri-apps/api/core", () => ({
  Channel: class {},
  invoke: vi.fn(async (cmd: string, args: any) => {
    if (cmd === "default_shell") return ["pwsh.exe", ["-NoLogo"]];
    if (cmd !== "daemon_call" && cmd !== "daemon_call_bg") return null;
    calls.push({ method: args.method, params: args.params });
    switch (args.method) {
      case "session.pending":
        return pending;
      case "session.list":
        return live;
      case "settings.get":
        return { rules: `rules:${args.params.terse ?? "global"}` };
      case "app.dir":
        return { dir: "C:/data" };
      case "session.spawn":
        if (failSpawnFor && args.params.label === failSpawnFor) throw new Error("no such worktree");
        return { id: calls.length };
      default:
        return {};
    }
  }),
}));

import { setProjectPref } from "./store";
import { agentLine, restoreSessions, shQuote } from "./api";

describe("shQuote", () => {
  it("doubles single quotes for PowerShell, typographic ones too", () => {
    expect(shQuote("it's", true)).toBe("'it''s'");
    expect(shQuote("a\u2019b", true)).toBe("'a\u2019\u2019b'");
  });
  it("closes, escapes and reopens the string for sh", () => {
    expect(shQuote("it's", false)).toBe("'it'\\''s'");
  });
  it("leaves a plain path alone", () => {
    expect(shQuote("C:/Users/x/hooks.json", true)).toBe("'C:/Users/x/hooks.json'");
  });
});

describe("agentLine", () => {
  const hooks = "C:/d/hooks.json";
  it("starts claude fresh with a session id", () => {
    expect(agentLine("claude", "abc", false, true, hooks)).toBe("claude --settings 'C:/d/hooks.json' --session-id abc");
  });
  it("starts other agents by name", () => {
    expect(agentLine("codex", undefined, false, false, hooks)).toBe("codex");
  });
  it("resumes with a fallback to a fresh start, in each shell's syntax", () => {
    expect(agentLine("claude", "abc", true, true, hooks)).toBe(
      "claude --settings 'C:/d/hooks.json' --resume abc; if (!$?) { claude --settings 'C:/d/hooks.json' --session-id abc }",
    );
    expect(agentLine("claude", "abc", true, false, hooks)).toBe(
      "claude --settings 'C:/d/hooks.json' --resume abc || claude --settings 'C:/d/hooks.json' --session-id abc",
    );
    expect(agentLine("codex", undefined, true, false, hooks)).toBe("codex resume --last || codex");
  });
  it("survives a quote in the data dir and in the rules", () => {
    expect(agentLine("claude", "abc", false, true, "C:/o'brien/hooks.json", "be brief; it's fine")).toBe(
      "claude --settings 'C:/o''brien/hooks.json' --append-system-prompt 'be brief; it''s fine' --session-id abc",
    );
    expect(agentLine("claude", "abc", false, false, "/home/o'b/hooks.json")).toBe("claude --settings '/home/o'\\''b/hooks.json' --session-id abc");
  });
});

describe("restoreSessions", () => {
  beforeEach(() => {
    calls.length = 0;
    failSpawnFor = null;
    live = [];
  });

  it("restores in order, drops each entry right after it was dealt with, and reads settings once", async () => {
    pending = [
      { program: "pwsh.exe", label: "shell", cwd: "C:/w" },
      { program: "pwsh.exe", label: "agent-a", meta: { kind: "agent", agent: "claude", project: "p", worktree: "C:/w/a", key: "k1", resume: "r1" } },
      { program: "pwsh.exe", label: "agent-b", meta: { kind: "agent", agent: "claude", project: "p", worktree: "C:/w/b", key: "k2", resume: "r2" } },
    ];
    const r = await restoreSessions();
    expect(r).toEqual({ restored: 3, skipped: 0, failed: [] });
    const seq = calls.map((c) => c.method + (c.params.label ? `:${c.params.label}` : ""));
    expect(seq.filter((s) => s === "settings.get")).toHaveLength(1);
    const order = seq.filter((s) => s.startsWith("session.spawn") || s === "session.pending_remove");
    expect(order).toEqual([
      "session.spawn:shell",
      "session.pending_remove",
      "session.spawn:agent-a",
      "session.pending_remove",
      "session.spawn:agent-b",
      "session.pending_remove",
    ]);
  });

  it("drops bare shells (no cwd, no meta) without starting or counting them", async () => {
    pending = [{ program: "cmd", args: ["/q"] }, { program: "powershell" }, { program: "pwsh.exe", label: "shell", cwd: "C:/w" }];
    const r = await restoreSessions();
    expect(r).toEqual({ restored: 1, skipped: 0, failed: [] });
    expect(calls.filter((c) => c.method === "session.spawn")).toHaveLength(1);
    expect(calls.filter((c) => c.method === "session.pending_remove")).toHaveLength(3);
  });

  it("skips blank agents, reports failures, and still removes the failed entry", async () => {
    failSpawnFor = "bad";
    pending = [
      { program: "x", label: "blank", meta: { kind: "agent", agent: "claude", project: "p", worktree: "w", blank: true } },
      { program: "x", label: "bad", cwd: "C:/gone" },
      { program: "x", label: "ok", cwd: "C:/w" },
    ];
    const r = await restoreSessions();
    expect(r).toEqual({ restored: 1, skipped: 1, failed: ["bad"] });
    expect(calls.filter((c) => c.method === "session.pending_remove")).toHaveLength(3);
  });

  it("hands a restored agent its task file again", async () => {
    pending = [{ program: "pwsh.exe", label: "Tarefa 3", meta: { kind: "agent", agent: "claude", project: "p", worktree: "C:/w/a", key: "k", resume: "k", note: "C:/data/notes/p/Tarefa 3.md" } }];
    await restoreSessions();
    const spawn = calls.find((c) => c.method === "session.spawn")!;
    expect(spawn.params.label).toBe("Tarefa 3");
    expect(spawn.params.meta.note).toBe("C:/data/notes/p/Tarefa 3.md");
  });

  it("hands a restored lead back as the lead, and leaves a plain task alone", async () => {
    const ag = (label: string, extra: object) => ({ program: "pwsh.exe", label, meta: { kind: "agent", agent: "claude", project: "p", worktree: "C:/w/a", key: label, resume: label, ...extra } });
    pending = [ag("main", { lead: true }), ag("Tarefa 1", {})];
    await restoreSessions();
    const spawns = calls.filter((c) => c.method === "session.spawn");
    expect(spawns[0].params.label).toBe("main");
    expect(spawns[0].params.meta.lead).toBe(true);
    expect(spawns[1].params.meta.lead).toBeUndefined();
  });

  it("uses each project's own reply style and reads settings once per distinct level", async () => {
    setProjectPref("pt", { terse: "tiny" });
    setProjectPref("ps", { terse: "short" });
    const ag = (label: string, project: string) => ({ program: "pwsh.exe", label, meta: { kind: "agent", agent: "claude", project, worktree: `C:/w/${label}`, key: label, resume: label } });
    pending = [ag("a", "pt"), ag("b", "pt"), ag("c", "ps"), ag("d", "none"), ag("e", "none")];
    const r = await restoreSessions();
    expect(r.restored).toBe(5);
    const gets = calls.filter((c) => c.method === "settings.get").map((c) => c.params.terse);
    expect(gets.sort()).toEqual(["short", "tiny", undefined]);
    const line = (label: string) => calls.find((c) => c.method === "session.spawn" && c.params.label === label)!.params.args.join(" ");
    expect(line("a")).toContain("rules:tiny");
    expect(line("b")).toContain("rules:tiny");
    expect(line("c")).toContain("rules:short");
    expect(line("d")).toContain("rules:global");
    expect(line("e")).toContain("rules:global");
  });

  it("never starts a twin of an agent the daemon already runs, but still drops its pending entry", async () => {
    const ag = (label: string, key: string) => ({ program: "pwsh.exe", label, meta: { kind: "agent", agent: "claude", project: "p", worktree: "C:/w/a", key, resume: key } });
    pending = [ag("a", "k1"), ag("b", "k2")];
    live = [{ id: 1, running: true, label: "a", meta: { kind: "agent", key: "k1" } }];
    const r = await restoreSessions();
    expect(r).toEqual({ restored: 1, skipped: 1, failed: [] });
    expect(calls.filter((c) => c.method === "session.spawn").map((c) => c.params.label)).toEqual(["b"]);
    expect(calls.filter((c) => c.method === "session.pending_remove")).toHaveLength(2);
  });

  it("a dead session with the same key does not block the restore", async () => {
    pending = [{ program: "pwsh.exe", label: "a", meta: { kind: "agent", agent: "claude", project: "p", worktree: "C:/w/a", key: "k1", resume: "k1" } }];
    live = [{ id: 1, running: false, label: "a", meta: { kind: "agent", key: "k1" } }];
    expect((await restoreSessions()).restored).toBe(1);
  });
});
