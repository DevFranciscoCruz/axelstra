import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, waitFor } from "@solidjs/testing-library";
import { createComponent } from "solid-js";

const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => invoke(...a), Channel: class {} }));

beforeEach(() => invoke.mockReset());
afterEach(() => cleanup());

const asDaemon = (answers: Record<string, unknown>) =>
  invoke.mockImplementation(async (_cmd: string, args?: { method: string }) => {
    if (!args?.method) return null; // host commands other than daemon_call
    if (args.method in answers) return answers[args.method];
    throw new Error(`unknown method ${args.method}`);
  });

describe("daemon method names the frontend and the daemon agree on", () => {
  it("calls notices.take, backup.export, backup.restore, orch.trust_info and env.tools", async () => {
    const api = await import("./api");
    asDaemon({
      "notices.take": { notices: ["x"] },
      "backup.export": { path: "/b" },
      "backup.restore": { restored: ["settings.json"] },
      "orch.trust_info": { pin: "p", cards: [] },
      "env.tools": { tools: [{ id: "git", found: true, path: "/g", version: "2.1.0" }] },
    });
    expect(await api.storeNotices()).toEqual(["x"]);
    expect(await api.backupExport()).toEqual({ path: "/b" });
    expect(await api.backupRestore("/b")).toEqual({ restored: ["settings.json"] });
    expect((await api.orchTrustInfo("p"))?.pin).toBe("p");
    expect((await api.envTools())?.[0].version).toBe("2.1.0");
    const methods = invoke.mock.calls.map((c) => c[1]?.method);
    expect(methods).toEqual(["notices.take", "backup.export", "backup.restore", "orch.trust_info", "env.tools"]);
  });

  it("an older daemon without them degrades instead of failing", async () => {
    const api = await import("./api");
    asDaemon({});
    expect(await api.storeNotices()).toEqual([]);
    expect(await api.envTools()).toBeNull();
    expect(await api.backupExport()).toBeNull();
    expect(await api.orchTrustInfo("p")).toBeNull();
  });
});

describe("Welcome shows what is missing", () => {
  it("marks found tools with their version and missing ones as missing", async () => {
    const { Welcome } = await import("./Welcome");
    asDaemon({
      "env.tools": {
        tools: [
          { id: "git", found: true, path: "/g", version: "2.45.1" },
          { id: "claude", found: false, path: null, version: null },
          { id: "gh", found: true, path: "/gh", version: null },
        ],
      },
    });
    const r = render(() => createComponent(Welcome, { onAddProject: vi.fn() }));
    await waitFor(() => expect(r.container.querySelector('[data-tool-state="missing"]')).not.toBeNull());
    const state = (name: string) => [...r.container.querySelectorAll("li[data-tool-state]")].find((li) => li.textContent?.startsWith(name))!;
    expect(state("git").getAttribute("data-tool-state")).toBe("found");
    expect(state("git").textContent).toContain("2.45.1");
    expect(state("claude").getAttribute("data-tool-state")).toBe("missing");
    expect(state("gh").getAttribute("data-tool-state")).toBe("found");
  });

  it("keeps the plain hints when the daemon cannot say", async () => {
    const { Welcome } = await import("./Welcome");
    asDaemon({});
    const r = render(() => createComponent(Welcome, { onAddProject: vi.fn() }));
    await waitFor(() => expect(invoke).toHaveBeenCalled());
    const items = [...r.container.querySelectorAll("li[data-tool-state]")];
    expect(items.length).toBe(3);
    expect(items.every((li) => li.getAttribute("data-tool-state") === "unknown")).toBe(true);
  });
});
