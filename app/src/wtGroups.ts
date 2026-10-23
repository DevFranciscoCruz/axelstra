/** Pure helpers for the sidebar's worktree list: which worktrees are finished, and how the rest nest under their parent. */

export type GroupWt = { path: string; branch: string | null; is_main: boolean; parent?: string | null };

const key = (p: string | null | undefined) => (p ?? "").replace(/\\/g, "/").replace(/\/+$/, "").toLowerCase();

export type Row<W extends GroupWt> = {
  wt: W;
  /** 0 = top level */
  depth: number;
  /** direct children shown under this row (finished ones are not counted: they live in Concluídos) */
  kids: number;
  /** this row and everything below it, to sum the agents of a whole group */
  members: string[];
};

/**
 * A worktree is finished when it is not the main one, the daemon lists its branch as merged (clean tree, nobody working
 * in it) and no session runs in it right now (the daemon's answer can be a few seconds old).
 */
export function isFinished(w: GroupWt, merged: ReadonlySet<string>, running: number): boolean {
  return !w.is_main && running === 0 && merged.has(key(w.path));
}

/**
 * The visible rows of a project, parents before their children. Main first, then pinned, then the order given.
 * `collapsed` holds normalized paths of parents whose children are folded away. A child whose parent is missing, hidden
 * or finished is shown at the top level, so nothing disappears. Cycles in the data cannot hang it.
 */
export function layout<W extends GroupWt>(
  all: W[],
  opts: { finished: ReadonlySet<string>; hidden?: ReadonlySet<string>; collapsed?: ReadonlySet<string>; pinned?: (path: string) => boolean },
): { rows: Row<W>[]; finished: W[] } {
  const hidden = opts.hidden ?? new Set<string>();
  const collapsed = opts.collapsed ?? new Set<string>();
  const isDone = (w: W) => !w.is_main && opts.finished.has(key(w.path));
  const finished = all.filter(isDone);
  const shown = all.filter((w) => !hidden.has(key(w.path)) && !isDone(w));
  const byKey = new Map(shown.map((w) => [key(w.path), w]));
  const parentOf = (w: W) => {
    const par = key(w.parent);
    return par && par !== key(w.path) && byKey.has(par) ? par : "";
  };
  const kidsOf = new Map<string, W[]>();
  const top: W[] = [];
  for (const w of shown) {
    const par = parentOf(w);
    if (par) kidsOf.set(par, [...(kidsOf.get(par) ?? []), w]);
    else top.push(w);
  }
  const pinned = opts.pinned ?? (() => false);
  top.sort((a, b) => Number(b.is_main) - Number(a.is_main) || Number(pinned(b.path)) - Number(pinned(a.path)));
  const rows: Row<W>[] = [];
  const seen = new Set<string>();
  const members = (w: W, trail: Set<string>): string[] => {
    const k = key(w.path);
    if (trail.has(k)) return [];
    const t = new Set(trail).add(k);
    return [k, ...(kidsOf.get(k) ?? []).flatMap((c) => members(c, t))];
  };
  const walk = (w: W, depth: number) => {
    const k = key(w.path);
    if (seen.has(k)) return;
    seen.add(k);
    const kids = (kidsOf.get(k) ?? []).filter((c) => !seen.has(key(c.path)));
    const all = members(w, new Set());
    rows.push({ wt: w, depth, kids: kids.length, members: all });
    if (!collapsed.has(k)) for (const c of kids) walk(c, depth + 1);
    // folded away, not lost: the loop check below must not list them
    else all.forEach((m) => seen.add(m));
  };
  for (const w of top) walk(w, 0);
  // a loop of parents (hand edited file) has no top-level entry: show those worktrees flat instead of losing them
  for (const w of shown) {
    if (!seen.has(key(w.path))) walk({ ...w, parent: null }, 0);
  }
  return { rows, finished };
}

/** Number of running agents in a group, given each member's running count (members from `Row.members`). */
export const groupRunning = (members: string[], running: (key: string) => number): number => members.reduce((n, m) => n + running(m), 0);

export { key as wtKey };
