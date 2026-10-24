import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createComponent } from "solid-js";

const h = vi.hoisted(() => ({ toasts: [] as { text: string; action?: { label: string; run: () => void } }[] }));

const card = {
  meta: {
    id: "k1",
    title: "Fix it",
    status: "todo",
    agent: "claude",
    depends_on: ["k0"],
    priority: 2,
    acceptance: null,
  },
  body: "b",
};
const auto = {
  id: "a1",
  name: "Nightly",
  project: "p",
  agent: "claude",
  prompt: "x",
  schedule: { kind: "daily", time: "09:00" },
  enabled: true,
  last_run: 0,
};

vi.mock("./api", () => ({
  kanbanList: vi.fn(async () => ({ cards: [card], broken: [] })),
  kanbanMove: vi.fn(),
  kanbanSave: vi.fn(async () => ({})),
  kanbanDelete: vi.fn(async () => ({})),
  orchStatus: vi.fn(async () => ({ state: "idle" })),
  orchRetry: vi.fn(),
  orchStart: vi.fn(),
  orchTrustInfo: vi.fn(async () => null),
  agentsList: vi.fn(async () => []),
  agentOptions: () => [],
  agentWarning: () => null,
  gitPr: vi.fn(),
  gitPush: vi.fn(),
  listAutomations: vi.fn(async () => [auto]),
  saveAutomation: vi.fn(async (a: unknown) => a),
  deleteAutomation: vi.fn(async () => ({})),
  runAutomation: vi.fn(),
}));
vi.mock("./store", () => ({
  refreshProjects: vi.fn(async () => undefined),
  refreshSessions: vi.fn(async () => undefined),
  selectWorktree: vi.fn(),
  setError: vi.fn(),
  setShowKanban: vi.fn(),
  setShowAutomations: vi.fn(),
  showNotice: vi.fn(),
  projects: () => [{ id: "p", name: "Proj", worktrees: [] }],
  toast: (t: { text: string; action?: { label: string; run: () => void } }) => h.toasts.push(t),
}));
vi.mock("./Confirm", () => ({ ask: vi.fn(async () => true), askDiscard: vi.fn(async () => true) }));

import * as api from "./api";
import { Kanban } from "./Kanban";
import { Automations } from "./Automations";

const settle = async () => {
  for (let i = 0; i < 15; i++) await Promise.resolve();
};
const byLabel = (el: HTMLElement, label: string) => el.querySelector<HTMLElement>(`[aria-label="${label}"]`)!;

afterEach(() => {
  cleanup();
  h.toasts.length = 0;
  vi.clearAllMocks();
});

describe("delete with undo", () => {
  it("Kanban: deleting a card toasts 'Desfazer', which saves it back under the same id", async () => {
    const r = render(() => createComponent(Kanban, { project: "p" }));
    await settle();
    fireEvent.click(r.container.querySelector(".kcard")!);
    await settle();
    const del = [...document.querySelectorAll("button")].find((b) => b.textContent?.trim() === "Apagar")!;
    fireEvent.click(del);
    await settle();
    expect(api.kanbanDelete).toHaveBeenCalledWith("p", "k1");
    const t = h.toasts.at(-1)!;
    expect(t.action?.label).toBe("Desfazer");
    t.action!.run();
    await settle();
    expect(api.kanbanSave).toHaveBeenCalledWith(
      "p",
      expect.objectContaining({ id: "k1", title: "Fix it", depends_on: ["k0"], priority: 2 }),
    );
  });

  it("Automations: deleting toasts 'Desfazer', which saves the same object again", async () => {
    const r = render(() => createComponent(Automations, {}));
    await settle();
    fireEvent.click(byLabel(document.body, "Apagar automatização") ?? r.container);
    await settle();
    expect(api.deleteAutomation).toHaveBeenCalledWith("a1");
    const t = h.toasts.at(-1)!;
    expect(t.action?.label).toBe("Desfazer");
    t.action!.run();
    await settle();
    expect(api.saveAutomation).toHaveBeenCalledWith(auto);
  });
});
