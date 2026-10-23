/** Opening tags (up to their closing `>`) of every `<name ...>` in a TSX source. */
export const openTags = (src: string, name: string) => {
  const out: { line: number; open: string }[] = [];
  let i = 0;
  while ((i = src.indexOf(`<${name}`, i)) >= 0) {
    if (!/[\s>]/.test(src[i + name.length + 1])) {
      i += name.length + 1;
      continue;
    }
    let j = i + name.length + 1;
    let depth = 0;
    let quote: string | null = null;
    for (; j < src.length; j++) {
      const c = src[j];
      if (quote) {
        if (c === quote) quote = null;
        continue;
      }
      if (c === '"' || c === "'" || c === "`") quote = c;
      else if (c === "{") depth++;
      else if (c === "}") depth--;
      else if (c === ">" && depth === 0) break;
    }
    out.push({ line: src.slice(0, i).split("\n").length, open: src.slice(i, j + 1) });
    i = j;
  }
  return out;
};

/** WCAG contrast ratio of two #rrggbb colours. */
export const contrast = (a: string, b: string) => {
  const lum = (h: string) => {
    const [r, g, bl] = [1, 3, 5].map((k) => parseInt(h.slice(k, k + 2), 16) / 255).map((c) => (c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4));
    return 0.2126 * r + 0.7152 * g + 0.0722 * bl;
  };
  const [hi, lo] = [lum(a), lum(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
};
