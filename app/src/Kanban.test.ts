import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createComponent } from "solid-js";

const h = vi.hoisted(() => ({
  trust: null as null | {
    pin: string;
    cards: { id: string; agent: string; permission_mode: string; bypass: boolean; acceptance: string | null }[];
  },
  answer: true,
  asked: [] as { message: string; opts: Record<string, unknown> }[],
  started: [] as unknown[][],
}));

const card = {
  meta: { id: "k1", title: "Fix it", status: "todo", agent: "claude", depends_on: [], priority: 0, acceptance: null },
  body: "b",
};

vi.mock("./api", () => ({
  kanbanList: vi.fn(async () => ({ cards: [card], broken: [] })),
  kanbanMove: vi.fn(),
  kanbanSave: vi.fn(),
  kanbanDelete: vi.fn(),
  orchStatus: vi.fn(async () => ({ state: "idle" })),
  orchRetry: vi.fn(),
  orchStart: vi.fn(async (...a: unknown[]) => {
    h.started.push(a);
    return { state: "running" };
  }),
  orchTrustInfo: vi.fn(async () => h.trust),
  agentsList: vi.fn(async () => []),
  agentOptions: () => [],
  agentWarning: () => null,
  gitPr: vi.fn(),
  gitPush: vi.fn(),
}));
vi.mock("./store", () => ({
  refreshProjects: vi.fn(async () => undefined),
  selectWorktree: vi.fn(),
  setError: vi.fn(),
  setShowKanban: vi.fn(),
  showNotice: vi.fn(),
  toast: vi.fn(),
}));
vi.mock("./Confirm", () => ({
  ask: vi.fn(async (message: string, opts: Record<string, unknown>) => {
    h.asked.push({ message, opts });
    return h.answer;
  }),
}));

import { Kanban } from "./Kanban";

const settle = async () => {
  for (let i = 0; i < 10; i++) await Promise.resolve();
};

beforeEach(() => {
  h.trust = null;
  h.answer = true;
  h.asked.length = 0;
  h.started.length = 0;
});
afterEach(() => cleanup());

const startButton = (el: HTMLElement) => el.querySelector<HTMLButtonElement>(".kbar button.primary")!;

describe("Kanban start orchestration", () => {
  it("bypass cards need the typed opt-in and the pin is sent", async () => {
    h.trust = {
      pin: "pin123",
      cards: [{ id: "k1", agent: "claude", permission_mode: "bypassPermissions", bypass: true, acceptance: null }],
    };
    const r = render(() => createComponent(Kanban, { project: "p" }));
    await settle();
    fireEvent.click(startButton(r.container));
    await settle();
    expect(h.asked).toHaveLength(1);
    expect(h.asked[0].message).toContain("bypassPermissions");
    expect(h.asked[0].opts.typeToConfirm).toBe("bypass");
    expect(h.started[0]).toEqual(["p", 3, false, "pin123"]);
  });

  it("declining sends nothing", async () => {
    h.trust = {
      pin: "x",
      cards: [{ id: "k1", agent: "claude", permission_mode: "default", bypass: false, acceptance: null }],
    };
    h.answer = false;
    const r = render(() => createComponent(Kanban, { project: "p" }));
    await settle();
    fireEvent.click(startButton(r.container));
    await settle();
    expect(h.started).toHaveLength(0);
  });

  it("an old daemon without trust info still asks and starts without a pin", async () => {
    const r = render(() => createComponent(Kanban, { project: "p" }));
    await settle();
    fireEvent.click(startButton(r.container));
    await settle();
    expect(h.asked[0].opts.typeToConfirm).toBeUndefined();
    expect(h.started[0]).toEqual(["p", 3, false, undefined]);
  });
});
