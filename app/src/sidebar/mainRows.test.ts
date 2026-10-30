import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createComponent } from "solid-js";
import type { SessionInfo } from "../api";

const h = vi.hoisted(() => ({
  sessions: [] as SessionInfo[],
  active: null as number | null,
  selected: null as string | null,
  opened: [] as [string, number][],
}));

vi.mock("@tauri-apps/api/core", () => ({ Channel: class {}, invoke: async () => undefined }));
vi.mock("../tauriEvents", () => ({ onTauriEvent: () => () => undefined }));
vi.mock("../store", async (orig) => ({
  ...(await orig<typeof import("../store")>()),
  sessionsOf: () => h.sessions,
  sessionsLoaded: () => true,
  activeSession: () => h.active,
  selectedWorktree: () => h.selected,
  openSession: (path: string, id: number) => void h.opened.push([path, id]),
}));

import { WorktreeRow } from "./WorktreeRow";
import { agentGroups, allowManualAgent, manualAgentPlan, worktreeRowActive } from "./mainRules";

const MAIN = "/p/cafe";
const agent = (id: number, label: string, meta: Record<string, unknown> = {}): SessionInfo =>
  ({ id, label, running: true, meta: { kind: "agent", worktree: MAIN, ...meta } }) as unknown as SessionInfo;
const mainSeat = agent(1, "main", { main: true });
const task = agent(2, "Tarefa 1");

const renderRow = (isMain: boolean, onOpen = vi.fn()) => {
  render(() =>
    createComponent(WorktreeRow, {
      project: { id: "p", default_branch: "main" },
      wt: { path: MAIN, branch: "main", is_main: isMain },
      depth: 0,
      kids: 0,
      folded: false,
      groupLive: 0,
      topic: undefined,
      overlap: null,
      overlapCount: 0,
      agentsHidden: false,
      menu: () => [],
      onOpen,
      onToggleFold: () => undefined,
      onToggleAgents: () => undefined,
      onStartLead: () => undefined,
      onDrop: () => undefined,
    }),
  );
  return onOpen;
};
const agentRow = (id: number) => document.querySelector<HTMLElement>(`[data-agent-row="${id}"]`)!;
const wtButton = () => document.querySelector(".item.wt .name")!.closest("button")!;

beforeEach(() => {
  h.sessions = [mainSeat, task];
  h.active = null;
  h.selected = null;
  h.opened = [];
});
afterEach(cleanup);

describe("sidebar: main row and task rows", () => {
  it("clicking a Tarefa opens exactly that task and does not reach the worktree row", () => {
    const onOpen = renderRow(false);
    fireEvent.click(agentRow(2));
    expect(h.opened).toEqual([[MAIN, 2]]);
    expect(onOpen).not.toHaveBeenCalled();
  });

  it("clicking the main row selects the worktree and opens no task", () => {
    const onOpen = renderRow(true);
    fireEvent.click(wtButton());
    expect(onOpen).toHaveBeenCalledTimes(1);
    expect(h.opened).toEqual([]);
  });

  it("clicking the main session row opens the main, not a task", () => {
    renderRow(true);
    fireEvent.click(agentRow(1));
    expect(h.opened).toEqual([[MAIN, 1]]);
  });

  it("the main checkout renders no Tarefa as a task: old ones are marked legacy and still open", () => {
    renderRow(true);
    expect(document.querySelector('[data-agent-kind="task"]')).toBeNull();
    const legacy = document.querySelector<HTMLElement>('[data-agent-kind="legacy"]')!;
    expect(legacy).not.toBeNull();
    expect(legacy.classList.contains("legacy")).toBe(true);
    expect(legacy.textContent).toContain("legado");
    expect(agentRow(2).title).toContain("worktree");
    expect(document.querySelector('[data-agent-kind="seat"] [data-agent-row="1"]')).not.toBeNull();
    fireEvent.click(agentRow(2));
    expect(h.opened).toEqual([[MAIN, 2]]);
  });

  it("a worktree shows its tasks as tasks, after the lead", () => {
    h.sessions = [task, agent(3, "main", { lead: true })];
    renderRow(false);
    const kinds = [...document.querySelectorAll<HTMLElement>("[data-agent-kind]")].map((e) => e.dataset.agentKind);
    expect(kinds).toEqual(["seat", "task"]);
  });

  it("the worktree row is highlighted for the worktree or its main, not while a task is active", () => {
    h.selected = MAIN;
    h.active = 2;
    renderRow(true);
    expect(document.querySelector(".item.wt")!.classList.contains("active")).toBe(false);
    expect(agentRow(2).closest(".wt-agent")!.classList.contains("active")).toBe(true);
  });
});

describe("mainRules", () => {
  it("agentGroups: main checkout = seat + legacy, worktree = lead + tasks", () => {
    expect(agentGroups([task, mainSeat], true)).toEqual({ seats: [mainSeat], tasks: [], legacy: [task] });
    const lead = agent(3, "main", { lead: true });
    expect(agentGroups([task, lead], false)).toEqual({ seats: [lead], tasks: [task], legacy: [] });
  });

  it("worktreeRowActive: none, terminal or main active -> true; task -> false; other worktree -> false", () => {
    const shell = { id: 9, label: "sh", running: true, meta: { kind: "shell" } } as unknown as SessionInfo;
    const all = [mainSeat, task, shell];
    expect(worktreeRowActive(MAIN, MAIN, null, all)).toBe(true);
    expect(worktreeRowActive(MAIN, MAIN, 1, all)).toBe(true);
    expect(worktreeRowActive(MAIN, MAIN, 9, all)).toBe(true);
    expect(worktreeRowActive(MAIN, MAIN, 2, all)).toBe(false);
    expect(worktreeRowActive("/p/other", MAIN, null, all)).toBe(false);
  });

  it("a manual agent in main with a main running suggests a worktree and starts nothing", () => {
    const notify = vi.fn();
    const newWorktree = vi.fn();
    const ok = allowManualAgent({ project: { id: "p" }, wt: { path: MAIN, is_main: true } }, [mainSeat], {
      notify,
      newWorktree,
    });
    expect(ok).toBe(false);
    expect(notify).toHaveBeenCalledTimes(1);
    const [, action] = notify.mock.calls[0];
    action.run();
    expect(newWorktree).toHaveBeenCalledWith("p");
  });

  it("a manual agent spawns in main without a running main, and always in a worktree", () => {
    expect(manualAgentPlan({ is_main: true }, [task])).toBe("spawn");
    expect(manualAgentPlan({ is_main: true }, [{ ...mainSeat, running: false }])).toBe("spawn");
    expect(manualAgentPlan({ is_main: false }, [mainSeat, task])).toBe("spawn");
    expect(manualAgentPlan({ is_main: true }, [mainSeat])).toBe("worktree");
  });
});
