import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, render } from "@solidjs/testing-library";
import { createComponent } from "solid-js";

vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(() => Promise.resolve(() => {})) }));
vi.mock("@tauri-apps/api/core", () => ({
  Channel: class {},
  invoke: async (cmd: string, args?: { method?: string }) => {
    if (cmd === "hosts_list") {
      return [{ id: "h1", name: "Servidor", spec: "me@srv", slot: 1, state: "connected" }];
    }
    if (cmd !== "daemon_call" && cmd !== "daemon_call_bg") return undefined;
    switch (args?.method) {
      case "settings.get":
        return {
          terse: "off",
          rules: null,
          pm: null,
          installed: [],
          auto_install: false,
          notify_sound: true,
          notify_mode: "all",
          shell: "auto",
          ui: {},
          budgets: {},
        };
      case "agents.list":
        return [
          { id: "claude", name: "Claude Code", installed: true },
          { id: "gemini", name: "Gemini CLI", installed: true },
        ];
      case "hooks.status":
        return { installed: false, path: "p" };
      default:
        return args?.method?.includes("list") ? [] : undefined;
    }
  },
}));

import { Settings } from "./Settings";
import { loadHosts } from "./store";
import { P } from "./strings.panels";
import { R } from "./strings.remote";

const settle = async () => {
  for (let i = 0; i < 40; i++) await new Promise((r) => setTimeout(r, 0));
};

describe("Settings: remote hosts and per-agent hooks", () => {
  afterEach(cleanup);
  it("shows a remote host and the Gemini hooks block together", async () => {
    await loadHosts();
    render(() => createComponent(Settings, {}));
    await settle();
    const text = document.body.textContent ?? "";
    expect(text).toContain(R.section);
    expect(text).toContain("Servidor");
    expect(text).toContain(P.hooks.title);
    expect(text).toContain(P.hooks.install("Gemini CLI"));
    expect(text).toContain(P.hooks.install("Claude Code"));
  });
});
