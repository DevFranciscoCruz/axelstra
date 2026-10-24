/** Terminals of the last few tabs stay mounted (hidden); these are the pure rules for which ones. */
export const KEEP = 5;

/** `kept` after `id` became the active tab: it is added, and the least recently used one goes when there are more than `keep`. */
export const touchKept = (kept: number[], id: number, lastUsed: ReadonlyMap<number, number>, keep = KEEP): number[] => {
  const next = kept.includes(id) ? kept : [...kept, id];
  if (next.length <= keep) return next;
  const oldest = next.filter((x) => x !== id).sort((a, b) => (lastUsed.get(a) ?? 0) - (lastUsed.get(b) ?? 0))[0];
  return next.filter((x) => x !== oldest);
};

/** `kept` without the ids that no longer exist (the same array when nothing was closed); closed ids also leave `lastUsed`. */
export const pruneKept = (kept: number[], alive: ReadonlySet<number>, lastUsed: Map<number, number>): number[] => {
  for (const id of lastUsed.keys()) if (!alive.has(id)) lastUsed.delete(id);
  return kept.some((id) => !alive.has(id)) ? kept.filter((id) => alive.has(id)) : kept;
};
