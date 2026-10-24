import type { BudgetEvent } from "./api";
import { pushNotification } from "./notifications";
import { B } from "./strings.budget";
import { toast } from "./toasts";
import { fmtTokens } from "./usageFormat";

export const budgetText = (e: BudgetEvent, nameOf: (path: string) => string): string => {
  const name = nameOf(e.worktree);
  if (e.level === "warn") return B.warnText(e.period, name, e.pct, fmtTokens(e.used), fmtTokens(e.limit));
  return B.overText(e.period, name, fmtTokens(e.used), fmtTokens(e.limit)) + (e.paused?.length ? B.paused(e.paused.length) : "");
};

/** One toast and one bell entry per crossing; the daemon reports each level once per period. Returns how many were handled. */
export const handleBudgetEvents = (events: BudgetEvent[], nameOf: (path: string) => string): number => {
  for (const e of events) {
    const text = budgetText(e, nameOf);
    toast({ text, kind: e.level === "over" ? "error" : "warn", key: `budget:${e.worktree}:${e.period}:${e.level}`, record: false });
    pushNotification({ kind: "budget", worktree: e.worktree, text, at: e.at });
  }
  return events.length;
};
