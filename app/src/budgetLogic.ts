import type { BudgetLevel, BudgetLimit, BudgetRow, Budgets } from "./api";
import { norm } from "./paths";

/** Largest limit the daemon accepts. */
export const MAX_LIMIT = 1_000_000_000_000;

/**
 * A token amount typed by the user: `5M`, `800k`, `1,5M`, `1500000`, `1.500.000`.
 * `null` = empty (no limit), `undefined` = not a valid amount.
 */
export const parseTokens = (text: string): number | null | undefined => {
  const t = text.trim().replace(/\s/g, "");
  if (!t) return null;
  const grouped = /^\d{1,3}(\.\d{3})+$/.test(t) ? t.replace(/\./g, "") : t;
  const m = /^(\d+(?:[.,]\d+)?)([kmb])?$/i.exec(grouped);
  if (!m) return undefined;
  const suffix = m[2]?.toLowerCase();
  if (!suffix && /[.,]/.test(m[1])) return undefined;
  const mult = suffix === "k" ? 1e3 : suffix === "m" ? 1e6 : suffix === "b" ? 1e9 : 1;
  const n = Math.round(parseFloat(m[1].replace(",", ".")) * mult);
  return n >= 1 && n <= MAX_LIMIT ? n : undefined;
};

/** A limit as the field shows it again: `5M`, `800k`, else the whole number. */
export const formatLimit = (n: number | undefined): string => {
  if (!n) return "";
  if (n % 1e9 === 0) return `${n / 1e9}B`;
  if (n % 1e6 === 0) return `${n / 1e6}M`;
  if (n % 1e3 === 0) return `${n / 1e3}k`;
  return String(n);
};

export type Scope = "default" | { worktree: string };

/** `b` with `patch` merged into the limit of `scope`; a key set to `undefined` is removed. */
export const withLimit = (b: Budgets, scope: Scope, patch: Partial<Record<keyof BudgetLimit, number | boolean | undefined>>): Budgets => {
  const merge = (cur: BudgetLimit | undefined): BudgetLimit => {
    const next: Record<string, number | boolean> = { ...(cur ?? {}) } as Record<string, number | boolean>;
    for (const [k, v] of Object.entries(patch)) {
      if (v === undefined || (k === "auto_pause" && v === false)) delete next[k];
      else next[k] = v;
    }
    return next as BudgetLimit;
  };
  if (scope === "default") return { ...b, default: merge(b.default) };
  return { ...b, worktrees: { ...(b.worktrees ?? {}), [scope.worktree]: merge(b.worktrees?.[scope.worktree]) } };
};

/** `b` without the own limit of a worktree (it falls back to the default). */
export const withoutWorktree = (b: Budgets, worktree: string): Budgets => {
  const w = { ...(b.worktrees ?? {}) };
  for (const k of Object.keys(w)) if (norm(k) === norm(worktree)) delete w[k];
  return { ...b, worktrees: w };
};

export const hasLimits = (b: Budgets | undefined): boolean =>
  !!b && (!!b.default?.daily || !!b.default?.weekly || Object.keys(b.worktrees ?? {}).length > 0);

/** The rows that belong to a worktree. */
export const rowsFor = (rows: BudgetRow[], worktree: string): BudgetRow[] => rows.filter((r) => norm(r.worktree) === norm(worktree));

const RANK: Record<BudgetLevel, number> = { ok: 0, warn: 1, over: 2 };
export const worstLevel = (rows: BudgetRow[]): BudgetLevel => rows.reduce<BudgetLevel>((w, r) => (RANK[r.level] > RANK[w] ? r.level : w), "ok");

/** The meter colour classes the status bar already has (`warn`, `bad`). */
export const levelClass = (l: BudgetLevel): { warn: boolean; bad: boolean } => ({ warn: l === "warn", bad: l === "over" });
