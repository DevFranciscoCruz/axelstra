/** Rows of a long list are drawn this many at a time; "Mostrar mais" adds another page. */
export const PAGE = 300;

/** true when a folder listing came back the same (same names, same kinds, same order): nothing to redraw */
export const sameListing = (a: { name: string; dir: boolean }[] | null, b: { name: string; dir: boolean }[]): boolean =>
  a !== null && a.length === b.length && a.every((x, i) => x.name === b[i].name && x.dir === b[i].dir);

/** the part of `list` that is drawn for `limit` rows, and how many are held back */
export const capList = <T,>(list: T[], limit: number): { shown: T[]; hidden: number } => ({
  shown: list.length > limit ? list.slice(0, limit) : list,
  hidden: Math.max(0, list.length - limit),
});
