import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createComponent } from "solid-js";

const h = vi.hoisted(() => ({
  created: [] as string[],
  launched: [] as string[],
  leads: [] as (boolean | undefined)[],
  rejectCreate: false,
}));

vi.mock("@tauri-apps/api/core", () => ({
  Channel: class {},
  invoke: async (cmd: string, args?: { method?: string; params?: Record<string, unknown> }) => {
    if (cmd !== "daemon_call" && cmd !== "daemon_call_bg") return undefined;
    switch (args?.method) {
      case "project.list":
        return [
          {
            id: "p1",
            name: "Proj",
            path: "C:/proj",
            worktrees: [{ path: "C:/proj", branch: "main", head: "abc", is_main: true }],
          },
        ];
      case "dispatch.meta":
        return {};
      case "project.logo":
        return { data_url: null };
      case "worktree.status":
        return [];
      case "session.list":
        return [];
      case "worktree.create":
        if (h.rejectCreate) throw new Error("branch já existe");
        h.created.push(String(args.params?.name));
        return { path: `C:/proj-${args.params?.name}`, branch: String(args.params?.name) };
      default:
        return args?.method?.includes("list") ? [] : undefined;
    }
  },
}));
vi.mock("./api", async (orig) => ({
  ...(await orig<typeof import("./api")>()),
  launchDefaultAgent: vi.fn(async (...a: unknown[]) => {
    h.launched.push(a[2] as string);
    h.leads.push(a[7] as boolean | undefined);
  }),
}));
vi.mock("./Confirm", () => ({ ask: vi.fn(async () => true), askDiscard: vi.fn(async () => true) }));

import { Sidebar } from "./Sidebar";
import { refreshProjects, setCreatingWorktree, setInstalledAgents, error, setError } from "./store";

const settle = async () => {
  for (let i = 0; i < 40; i++) await new Promise((r) => setTimeout(r, 0));
};
const dialog = () => document.querySelector("div[aria-label='Novo worktree']");
const input = () => document.querySelector<HTMLInputElement>("input[placeholder^='nome do worktree']")!;
const create = () =>
  [...document.querySelectorAll("button")].find((b) => b.textContent?.includes("Criar worktree")) as HTMLButtonElement;

const pickAgent = async (agent: string) => {
  fireEvent.click(document.querySelector<HTMLElement>("[aria-label^='Agente:']")!);
  await settle();
  const names: Record<string, string> = { none: "Nenhum", claude: "Claude Code", codex: "Codex" };
  const opt = [...document.querySelectorAll<HTMLElement>("[role='option']")].find((o) =>
    o.textContent?.includes(names[agent]),
  )!;
  fireEvent.pointerDown(opt);
  fireEvent.pointerUp(opt);
  fireEvent.click(opt);
  await settle();
  expect(document.querySelector("[aria-label^='Agente:']")?.textContent).toContain(names[agent]);
};

const open = async (agent?: string) => {
  render(() => createComponent(Sidebar, {}));
  await refreshProjects();
  setCreatingWorktree("p1");
  await settle();
  expect(dialog()).not.toBeNull();
  if (agent) await pickAgent(agent);
};

beforeEach(() => {
  h.created.length = 0;
  h.launched.length = 0;
  h.leads.length = 0;
  h.rejectCreate = false;
  setInstalledAgents(["claude", "codex"]);
  setError("");
});
afterEach(() => {
  cleanup();
  setCreatingWorktree(null);
});

describe("B1: the Novo worktree dialog closes once the worktree is created", () => {
  for (const agent of ["none", "claude", "codex"]) {
    it(`click on Criar, agent ${agent}: dialog gone, created once`, async () => {
      await open(agent);
      fireEvent.input(input(), { target: { value: "feat-x" } });
      fireEvent.click(create());
      await settle();
      expect(dialog()).toBeNull();
      expect(h.created).toEqual(["feat-x"]);
      expect(h.launched).toEqual(agent === "none" ? [] : [agent]);
      // the agent of a worktree the user creates is that worktree's main, not a "Tarefa N"
      expect(h.leads).toEqual(agent === "none" ? [] : [true]);
    });
  }

  it("Enter in the name field: dialog gone, created once, and the Enter is consumed", async () => {
    await open("claude");
    fireEvent.input(input(), { target: { value: "feat-y" } });
    const ev = new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true });
    const field = input();
    field.dispatchEvent(ev);
    await settle();
    expect(ev.defaultPrevented).toBe(true);
    expect(dialog()).toBeNull();
    expect(h.created).toEqual(["feat-y"]);
  });

  it("a second Enter while it is being created does not create twice or reopen", async () => {
    await open("claude");
    fireEvent.input(input(), { target: { value: "feat-z" } });
    const field = input();
    fireEvent.keyDown(field, { key: "Enter" });
    fireEvent.keyDown(field, { key: "Enter" });
    await settle();
    expect(dialog()).toBeNull();
    expect(h.created).toEqual(["feat-z"]);
  });

  it("Enter opened from the + button: the browser's default Enter action must not click the + button the dialog gives focus back to", async () => {
    render(() => createComponent(Sidebar, {}));
    await refreshProjects();
    await settle();
    const plus = document.querySelector<HTMLElement>("button[aria-label='Novo worktree']")!;
    plus.focus();
    fireEvent.click(plus);
    await settle();
    expect(dialog()).not.toBeNull();
    const field = input();
    fireEvent.input(field, { target: { value: "feat-k" } });
    const ev = new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true });
    field.dispatchEvent(ev);
    // what the browser does after the keydown handlers: Enter activates the focused button
    if (!ev.defaultPrevented) (document.activeElement as HTMLElement).click();
    await settle();
    expect(dialog()).toBeNull();
    expect(h.created).toEqual(["feat-k"]);
  });

  it("createWorktree rejects: the dialog comes back with the name kept, the error is shown and nothing is launched", async () => {
    h.rejectCreate = true;
    await open("claude");
    fireEvent.input(input(), { target: { value: "dup" } });
    fireEvent.click(create());
    await settle();
    expect(dialog()).not.toBeNull();
    expect(input().value).toBe("dup");
    expect(error()).toContain("branch já existe");
    expect(h.launched).toEqual([]);
  });
});
