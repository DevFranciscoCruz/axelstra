import { describe, expect, it, vi } from "vitest";

const calls: { method: string; params: Record<string, unknown> }[] = [];
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(() => Promise.resolve(() => {})) }));
vi.mock("@tauri-apps/api/core", () => ({
  Channel: class {},
  invoke: vi.fn(async (cmd: string, args: { method: string; params: Record<string, unknown> }) => {
    if (cmd !== "daemon_call" && cmd !== "daemon_call_bg") return null;
    calls.push({ method: args.method, params: args.params });
    return { installed: false, path: "p" };
  }),
}));

import { hookAgentsToShow, hookSettingsPath, hooksInstall, hooksStatus, type AgentInfo } from "./api";
import { P } from "./strings.panels";

const info = (id: string, installed: boolean) => ({ id, name: id, installed }) as AgentInfo;

describe("hooks api", () => {
  it("defaults to claude and passes the agent param", async () => {
    await hooksStatus();
    await hooksStatus("gemini");
    await hooksInstall();
    await hooksInstall("gemini");
    expect(calls.map((c) => [c.method, c.params])).toEqual([
      ["hooks.status", { agent: "claude" }],
      ["hooks.status", { agent: "gemini" }],
      ["hooks.install", { agent: "claude" }],
      ["hooks.install", { agent: "gemini" }],
    ]);
  });
});

describe("hookAgentsToShow", () => {
  it("shows both when the list is unknown or empty", () => {
    expect(hookAgentsToShow(undefined)).toEqual(["claude", "gemini"]);
    expect(hookAgentsToShow([])).toEqual(["claude", "gemini"]);
  });
  it("hides gemini when it is not installed, never claude", () => {
    expect(hookAgentsToShow([info("claude", false), info("gemini", false)])).toEqual(["claude"]);
    expect(hookAgentsToShow([info("gemini", true)])).toEqual(["claude", "gemini"]);
  });
});

describe("hooks texts", () => {
  it("names the settings path and the agent", () => {
    expect(hookSettingsPath("claude")).toBe("~/.claude/settings.json");
    expect(hookSettingsPath("gemini")).toBe("~/.gemini/settings.json");
    expect(P.hooks.install("Gemini CLI")).toBe("Instalar hooks do Gemini CLI");
    expect(P.hooks.pathNote("~/.gemini/settings.json")).toContain("cópia de segurança antes de alterar");
  });
});
