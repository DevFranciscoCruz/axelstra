/** Ranking for the command palette: every word of the query must match; better matches come first. */

const fold = (s: string) =>
  s
    .toLowerCase()
    .normalize("NFD")
    .replace(/[\u0300-\u036f]/g, "");

/** how well one word matches `text` (both folded): 0 = no match */
const wordScore = (text: string, w: string): number => {
  if (text.startsWith(w)) return 100;
  const at = text.indexOf(w);
  if (at >= 0) {
    // right after a separator it starts a word of the label
    return /[\s/:\-_(]/.test(text[at - 1] ?? " ") ? 70 : 40;
  }
  // letters in order, nothing in between required: "dkr" finds "Docker"
  let i = 0;
  for (const c of text) if (c === w[i] && ++i === w.length) return 10;
  return 0;
};

/** score of `label` for the query `words` (already folded): 0 when some word does not match */
export const scoreLabel = (label: string, words: string[]): number => {
  const text = fold(label);
  let total = 0;
  for (const w of words) {
    const s = wordScore(text, w);
    if (!s) return 0;
    total += s;
  }
  // shorter labels say the same thing with less noise
  return total - Math.min(text.length, 60) / 100;
};

export type Ranked<T> = { item: T; score: number };

/**
 * Items matching `q`, best first. Items in `recents` (most recent first) get a bonus, which alone orders an empty
 * query: the recent ones first, then the rest in their given order.
 */
export const rank = <T extends { label: string; id: string }>(
  items: T[],
  q: string,
  recents: string[] = [],
  max = 200,
): T[] => {
  const words = fold(q).split(/\s+/).filter(Boolean);
  const bonus = (id: string) => {
    const i = recents.indexOf(id);
    return i < 0 ? 0 : 30 - i;
  };
  const out: (Ranked<T> & { at: number })[] = [];
  items.forEach((item, at) => {
    const base = words.length ? scoreLabel(item.label, words) : 1;
    if (base > 0) out.push({ item, score: base + (words.length ? bonus(item.id) / 2 : bonus(item.id) * 10), at });
  });
  out.sort((a, b) => b.score - a.score || a.at - b.at);
  return out.slice(0, max).map((r) => r.item);
};

/** `id` moved to the front of `list`, no repeats, at most `max` */
export const pushRecent = (list: string[], id: string, max = 8): string[] =>
  [id, ...list.filter((x) => x !== id)].slice(0, max);
