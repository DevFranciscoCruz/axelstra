/** Paths come back with either slash style; compare them in one normal form. The one `norm` for the whole app. */
export const norm = (p?: string | null) => (p ?? "").replace(/\\/g, "/").replace(/\/+$/, "").toLowerCase();

/** last segment of a path with either slash style; trailing slashes ignored ("C:\a\b\\" -> "b") */
export const basename = (p: string | null | undefined): string => (p ?? "").replace(/[\\/]+$/, "").split(/[\\/]/).pop() ?? "";

/** a copy of `set` with `v` removed when it was in, added when it was not (checkbox-style toggles on a Set held in a signal) */
export const toggleIn = <T>(set: ReadonlySet<T>, v: T): Set<T> => {
  const next = new Set(set);
  if (!next.delete(v)) next.add(v);
  return next;
};
