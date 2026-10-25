import { P } from "./strings.panels";
import type { KCard, OrchTrust, OrchTrustCard } from "./api";

/** `bypass*` modes (bypassPermissions, bypass-permissions…) run an agent with no supervision at all */
export const isBypass = (mode: string | null | undefined) => /^bypass/i.test((mode ?? "").trim());

/** cards the user must opt into explicitly: the daemon's own flag, or a bypass-looking mode */
export const bypassCards = (cards: OrchTrustCard[]) => cards.filter((c) => c.bypass || isBypass(c.permission_mode));

/** one line per card for the confirm text; bypass cards are marked so they stand out */
export const trustLines = (cards: OrchTrustCard[]) =>
  cards.map((c) => {
    const flag = c.bypass || isBypass(c.permission_mode) ? "⚠ " : "";
    return `${flag}${c.id}: ${c.agent} (${c.permission_mode || P.kanban.defaultMode})${c.acceptance ? P.kanban.acceptance(c.acceptance) : ""}`;
  });

export type StartPlan = {
  /** the confirm body */
  message: string;
  /** acceptance commands run in a shell, or a bypass card needs an explicit yes */
  warn: boolean;
  bypass: string[];
  trustAcceptance: boolean;
  /** hash of the cards the user saw: the daemon refuses if they change meanwhile */
  pin?: string;
};

/** What the user is asked and what is sent on yes. `trust` is null when the daemon has no `orch.trust_info`. */
export function planStart(n: number, trust: OrchTrust | null, legacyCmds: string[]): StartPlan {
  const head = P.kanban.planHead(n);
  if (!trust) {
    return {
      message: head + (legacyCmds.length ? P.kanban.legacyCmds(legacyCmds.join("\n")) : ""),
      warn: legacyCmds.length > 0,
      bypass: [],
      trustAcceptance: legacyCmds.length > 0,
    };
  }
  const bypass = bypassCards(trust.cards).map((c) => c.id);
  const cmds = trust.cards.filter((c) => c.acceptance).length;
  const bypassNote = bypass.length ? P.kanban.bypassNote(bypass.length, bypass.join(", ")) : "";
  return {
    message: `${head}\n\n${trustLines(trust.cards).join("\n")}${bypassNote}${cmds ? P.kanban.trustCmds : ""}`,
    warn: cmds > 0 || bypass.length > 0,
    bypass,
    trustAcceptance: cmds > 0,
    pin: trust.pin,
  };
}

/** the payload that re-creates a deleted card (same id, so dependents still point at it) */
export function restorePayload(c: KCard): Record<string, unknown> {
  return {
    id: c.meta.id,
    title: c.meta.title,
    body: c.body,
    status: c.meta.status,
    agent: c.meta.agent,
    priority: c.meta.priority,
    depends_on: c.meta.depends_on,
    ...(c.meta.acceptance ? { acceptance: c.meta.acceptance } : {}),
  };
}

// ---- drag and drop ----
// The daemon lists a board by priority (high first), then by id. Position inside a column is therefore the `priority`
// field of the card files, so a reorder is a handful of priority writes and the file format does not change.

export const COL_IDS = ["backlog", "todo", "in-progress", "needs-review", "done"];

/** the column a card is shown in: an unknown status is shown under the first one */
export const colOf = (status: string, cols: string[] = COL_IDS) => (cols.includes(status) ? status : cols[0]);

type Ord = { id: string; priority: number };

/** the daemon's order: priority descending, then id ascending */
export const cmpCards = (a: Ord, b: Ord) => b.priority - a.priority || (a.id < b.id ? -1 : a.id > b.id ? 1 : 0);

/** the cards of one column in board order */
export const columnCards = (cards: KCard[], col: string, cols: string[] = COL_IDS) =>
  cards.filter((c) => colOf(c.meta.status, cols) === col).sort((a, b) => cmpCards({ id: a.meta.id, priority: a.meta.priority }, { id: b.meta.id, priority: b.meta.priority }));

/** where a card would land: `col` and `index` among that column's cards without the dragged one (0..n) */
export type DropPos = { col: string; index: number };

/** insertion index for a pointer at `y`: how many of the (dragged-card-free) column's cards have their middle above it */
export const indexAt = (mids: number[], y: number) => mids.filter((m) => m < y).length;

/** the column under `x`, or null when the pointer is outside every column (`rects` are [left, right] per column) */
export const colAt = (rects: [number, number][], x: number): number | null => {
  const i = rects.findIndex(([l, r]) => x >= l && x <= r);
  return i < 0 ? null : i;
};

/** keyboard step of a picked-up card; `lens[i]` = cards in column i without the dragged one. Home/End jump to the ends. */
export function keyStep(pos: DropPos, key: string, cols: string[], lens: number[]): DropPos {
  const ci = Math.max(0, cols.indexOf(pos.col));
  const to = (c: number, index: number): DropPos => ({ col: cols[c], index: Math.max(0, Math.min(index, lens[c] ?? 0)) });
  switch (key) {
    case "ArrowUp":
      return to(ci, pos.index - 1);
    case "ArrowDown":
      return to(ci, pos.index + 1);
    case "Home":
      return to(ci, 0);
    case "End":
      return to(ci, lens[ci] ?? 0);
    case "ArrowLeft":
      return to(Math.max(0, ci - 1), pos.index);
    case "ArrowRight":
      return to(Math.min(cols.length - 1, ci + 1), pos.index);
    default:
      return { col: cols[ci], index: Math.max(0, Math.min(pos.index, lens[ci] ?? 0)) };
  }
}

/** where a card currently is, among the cards of its column without itself */
export function posOf(cards: KCard[], id: string, cols: string[] = COL_IDS): DropPos | null {
  const c = cards.find((x) => x.meta.id === id);
  if (!c) return null;
  const col = colOf(c.meta.status, cols);
  return { col, index: columnCards(cards, col, cols).findIndex((x) => x.meta.id === id) };
}

export type MovePlan = {
  /** the card is placed in this column (a kanban.move when its status differs) */
  status: string | null;
  /** priorities to write (kanban.save); only cards whose value changes */
  priorities: { id: string; priority: number }[];
};

/**
 * What to write to put card `id` at `pos`. The moved card keeps its priority when that already sorts it between its new
 * neighbours; otherwise it takes a value next to them; only when no value fits (equal priorities with ids in the wrong
 * order) the column is renumbered top-down, which leaves a stable strict order. Null when nothing changes.
 */
export function planMove(cards: KCard[], id: string, pos: DropPos, cols: string[] = COL_IDS): MovePlan | null {
  const m = cards.find((c) => c.meta.id === id);
  if (!m) return null;
  const from = colOf(m.meta.status, cols);
  const rest = columnCards(cards, pos.col, cols).filter((c) => c.meta.id !== id);
  const index = Math.max(0, Math.min(pos.index, rest.length));
  const moveCol = from !== pos.col || m.meta.status !== pos.col;
  const prev = rest[index - 1]?.meta;
  const next = rest[index]?.meta;
  const ord = (p: number): Ord => ({ id, priority: p });
  const fits = (p: number) => (!prev || cmpCards(prev, ord(p)) < 0) && (!next || cmpCards(ord(p), next) < 0);
  const cur = m.meta.priority;
  if (from === pos.col) {
    const old = columnCards(cards, from, cols).findIndex((c) => c.meta.id === id);
    if (old === index && !moveCol) return null;
  }
  const cand = [cur, prev?.priority, next?.priority, prev ? prev.priority - 1 : undefined, next ? next.priority + 1 : undefined].filter((p): p is number => p !== undefined);
  const p = cand.find(fits);
  if (p !== undefined) return { status: moveCol ? pos.col : null, priorities: p === cur ? [] : [{ id, priority: p }] };
  const order = [...rest.slice(0, index).map((c) => c.meta), m.meta, ...rest.slice(index).map((c) => c.meta)];
  const top = Math.max(...order.map((c) => c.priority));
  const priorities = order.map((c, i) => ({ id: c.id, priority: top - i })).filter((x) => x.priority !== order.find((c) => c.id === x.id)!.priority);
  return { status: moveCol ? pos.col : null, priorities };
}

/** the board as it will be once `plan` is written (shown at once, replaced by the daemon's answer) */
export function applyPlan(cards: KCard[], id: string, plan: MovePlan): KCard[] {
  return cards.map((c) => {
    const pr = plan.priorities.find((x) => x.id === c.meta.id);
    const status = c.meta.id === id && plan.status ? plan.status : c.meta.status;
    return pr || status !== c.meta.status ? { ...c, meta: { ...c.meta, status, priority: pr ? pr.priority : c.meta.priority } } : c;
  });
}

/** auto-scroll step for a pointer at `v` inside [lo, hi]: negative near `lo`, positive near `hi`, 0 in the middle; faster the closer to the edge */
export function edgeDelta(v: number, lo: number, hi: number, edge = 48, max = 18): number {
  if (hi - lo < edge * 2) return 0;
  if (v < lo + edge) return -Math.ceil((Math.min(edge, lo + edge - v) / edge) * max);
  if (v > hi - edge) return Math.ceil((Math.min(edge, v - (hi - edge)) / edge) * max);
  return 0;
}
