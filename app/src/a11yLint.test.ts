import { describe, expect, it } from "vitest";

/** Opening tags of every `<button>` in a TSX source, with the markup up to its `</button>`. */
export const buttons = (src: string) => {
  const out: { line: number; open: string; body: string }[] = [];
  let i = 0;
  while ((i = src.indexOf("<button", i)) >= 0) {
    if (!/[\s>]/.test(src[i + 7])) {
      i += 7;
      continue;
    }
    let j = i + 7;
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
    const end = src.indexOf("</button>", j);
    out.push({ line: src.slice(0, i).split("\n").length, open: src.slice(i, j + 1), body: src.slice(j + 1, end) });
    i = end;
  }
  return out;
};

/** True when the button shows only an icon (SVG, Icon component or a lone glyph), so it has no text name. */
export const iconOnly = (body: string) => {
  const rest = body
    .replace(/<\w*Icon\b[^>]*\/>/g, "\0")
    .replace(/<svg[\s\S]*?<\/svg>/g, "\0")
    .replace(/<\/?span\b[^>]*>/g, "")
    .replace(/[＋×✕]/g, "\0")
    .trim();
  return rest.includes("\0") && rest.replace(/\0/g, "").trim() === "";
};

describe("icon-only buttons", () => {
  const sources = import.meta.glob<string>("./*.tsx", { query: "?raw", import: "default", eager: true });

  it("detects icon-only markup", () => {
    expect(iconOnly(`\n  <Icon name="x" />\n`)).toBe(true);
    expect(iconOnly(`<span class="flip"><Icon name="chevDown" /></span>`)).toBe(true);
    expect(iconOnly(`<PanelIcon side="left" on={false} />`)).toBe(true);
    expect(iconOnly(`＋`)).toBe(true);
    expect(iconOnly(`<Icon name="check" /> Commit`)).toBe(false);
    expect(iconOnly(`Fechar`)).toBe(false);
  });

  it("every one has an aria-label", () => {
    const bad: string[] = [];
    for (const [f, src] of Object.entries(sources)) {
      for (const b of buttons(src)) {
        if (iconOnly(b.body) && !/aria-label(ledby)?=/.test(b.open)) bad.push(`${f}:${b.line}`);
      }
    }
    expect(bad).toEqual([]);
  });
});
