import { type Widen, count } from "./i18n";
import type { ptB } from "./strings.budget";
/** English texts of the token budgets (Settings section, status bar, popover, alerts). */

const period = { daily: "daily", weekly: "weekly" } as const;

export const enB: Widen<typeof ptB> = {
  title: "Token budgets",
  note: "Alerts when a worktree spends too many tokens in a day or a week: 80% is a warning, 100% is exceeded. Counts agents' input, output and cache writes.",
  defaultRow: "All worktrees",
  defaultHint: "Applies to worktrees without their own limit.",
  daily: "Daily limit",
  weekly: "Weekly limit",
  limitHint: "Tokens, for example 5M or 800k. Empty = no limit.",
  invalid: (v: string) => `“${v}” is not a valid token count. Enter for example 5M, 800k or 1500000.`,
  autoPause: "Interrupt agents when the limit is exceeded",
  autoPauseWarn:
    "Sends Esc to the agents working in the worktree when the limit is exceeded. It stops the current turn and may leave work half done. Off by default; it only interrupts if you turn it on here.",
  perWorktree: "Per-worktree limits",
  perWorktreeNote: "A worktree's own limit replaces the “All worktrees” one for that worktree.",
  addWorktree: "Add limit for…",
  addWorktreeLabel: "Worktree to give its own limit",
  remove: "Remove",
  removeLabel: (name: string) => `Remove the own limit of ${name}`,
  noWorktrees: "No worktrees to choose from.",
  oldDaemon: "The running daemon does not know budgets. Restart Axelstra to use them.",
  saved: "Budgets saved.",
  // status bar and popover
  periodName: (p: "daily" | "weekly") => period[p],
  meterTip: (p: "daily" | "weekly", name: string, used: string, limit: string, pct: number) =>
    `${name} ${period[p]} budget: ${used} of ${limit} tokens (${pct}%)`,
  meterShort: { daily: "day", weekly: "wk." },
  levelWord: { ok: "within the limit", warn: "near the limit", over: "exceeded" },
  chipOver: "Some token budgets are exceeded",
  chipWarn: "Some token budgets are near the limit",
  // alerts
  warnText: (p: "daily" | "weekly", name: string, pct: number, used: string, limit: string) =>
    `${name} ${period[p]} budget at ${pct}%: ${used} of ${limit} tokens`,
  overText: (p: "daily" | "weekly", name: string, used: string, limit: string) =>
    `${name} ${period[p]} budget exceeded: ${used} of ${limit} tokens`,
  paused: (n: number) => ` · ${count(n, "agent interrupted", "agents interrupted")}`,
};
