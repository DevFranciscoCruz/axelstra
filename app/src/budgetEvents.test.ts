import { beforeEach, describe, expect, it, vi } from "vitest";

const toast = vi.fn();
const pushNotification = vi.fn();
vi.mock("./toasts", () => ({ toast: (...a: unknown[]) => toast(...a) }));
vi.mock("./notifications", () => ({ pushNotification: (...a: unknown[]) => pushNotification(...a) }));

import type { BudgetEvent } from "./api";
import { budgetText, handleBudgetEvents } from "./budgetEvents";

const ev = (over: Partial<BudgetEvent> = {}): BudgetEvent => ({
  worktree: "/wt/a",
  period: "daily",
  level: "warn",
  used: 4_000_000,
  limit: 5_000_000,
  pct: 80,
  paused: [],
  at: 7,
  ...over,
});
const name = (p: string) => p.split("/").pop()!;

beforeEach(() => {
  toast.mockReset();
  pushNotification.mockReset();
});

describe("budgetText", () => {
  it("says the percentage for a warning", () => {
    expect(budgetText(ev(), name)).toBe("Orçamento diário de a a 80%: 4.0M de 5.0M tokens");
  });
  it("says exceeded for over, and how many agents were interrupted", () => {
    expect(budgetText(ev({ level: "over", period: "weekly", used: 6_000_000, pct: 120 }), name)).toBe(
      "Orçamento semanal de a excedido: 6.0M de 5.0M tokens",
    );
    expect(budgetText(ev({ level: "over", paused: [3] }), name)).toContain(" · 1 agente interrompido");
    expect(budgetText(ev({ level: "over", paused: [3, 4] }), name)).toContain(" · 2 agentes interrompidos");
  });
});

describe("handleBudgetEvents", () => {
  it("one toast and one bell entry per event, keyed so a repeat replaces", () => {
    const n = handleBudgetEvents([ev(), ev({ level: "over", period: "weekly" })], name);
    expect(n).toBe(2);
    expect(toast).toHaveBeenCalledTimes(2);
    expect(toast.mock.calls[0][0]).toMatchObject({ kind: "warn", key: "budget:/wt/a:daily:warn" });
    expect(toast.mock.calls[1][0]).toMatchObject({ kind: "error", key: "budget:/wt/a:weekly:over" });
    expect(pushNotification).toHaveBeenCalledWith(
      expect.objectContaining({ kind: "budget", worktree: "/wt/a", at: 7 }),
    );
  });
  it("nothing to do for no events", () => {
    expect(handleBudgetEvents([], name)).toBe(0);
    expect(toast).not.toHaveBeenCalled();
  });
});
