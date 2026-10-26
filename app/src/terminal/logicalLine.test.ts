import { describe, expect, it } from "vitest";
import { findPathLinks, findWebUrls } from "../pathlinks";
import { findHtmlLinks } from "./htmlLinks";
import { logicalLinks, readLogical, type BufferLike, type Match } from "./logicalLine";

const wide = (ch: string) => /[\u3000-\u9fff\uff00-\uffef]/.test(ch);

/** a fake buffer: `text` flows into rows of `cols` cells (wide chars take 2 and wrap whole), like xterm's soft wrap */
function fakeBuffer(text: string, cols: number): BufferLike {
  type C = { chars: string; w: number };
  const rows: C[][] = [[]];
  const used = () => rows[rows.length - 1].reduce((n, c) => n + (c.w || 0), 0);
  for (const ch of text) {
    const w = wide(ch) ? 2 : 1;
    if (used() + w > cols) rows.push([]);
    const row = rows[rows.length - 1];
    row.push({ chars: ch, w });
    if (w === 2) row.push({ chars: "", w: 0 });
  }
  return {
    getLine: (y) => {
      const r = rows[y];
      if (!r) return undefined;
      return {
        isWrapped: y > 0,
        length: cols,
        getCell: (x) =>
          x < r.length
            ? { getChars: () => r[x].chars, getWidth: () => r[x].w }
            : { getChars: () => "", getWidth: () => 1 },
      };
    },
  };
}

const paths = (buf: BufferLike, y: number) => {
  const find = (t: string): (Match & { path: string; line?: number; col?: number })[] =>
    findPathLinks(t).map((l) => ({ ...l, end: l.start + l.text.length }));
  return logicalLinks(buf, y, find);
};

describe("logicalLinks", () => {
  it("path on one line", () => {
    const [l] = paths(fakeBuffer("see src/a.ts:12 ok", 40), 1);
    expect(l.text).toBe("src/a.ts:12");
    expect(l.range).toEqual({ start: { x: 5, y: 1 }, end: { x: 15, y: 1 } });
  });

  it("Windows path split over 2 rows, in backticks", () => {
    const buf = fakeBuffer(
      "`" + String.raw`C:\Users\franc\target\release\bundle\nsis\Axelstra_0.1.0_x64-setup.exe` + "` done",
      40,
    );
    for (const y of [1, 2]) {
      const [l] = paths(buf, y);
      expect(l.text).toBe(String.raw`C:\Users\franc\target\release\bundle\nsis\Axelstra_0.1.0_x64-setup.exe`);
      expect(l.range.start).toEqual({ x: 2, y: 1 });
      expect(l.range.end.y).toBe(2);
    }
  });

  it("split over 3 rows, row 3 asked", () => {
    const buf = fakeBuffer(String.raw`x C:\aaaa\bbbb\cccc\dddd\eeee\ffff\gggg.txt y`, 14);
    const [l] = paths(buf, 3);
    expect(l.text).toBe(String.raw`C:\aaaa\bbbb\cccc\dddd\eeee\ffff\gggg.txt`);
    expect(l.range.start.y).toBe(1);
    expect(l.range.end.y).toBe(4);
  });

  it("relative path and trailing punctuation", () => {
    const buf = fakeBuffer("edit (crates/daemon/src/server.rs:42:7).", 20);
    const [l] = paths(buf, 2);
    expect(l.text).toBe("crates/daemon/src/server.rs:42:7");
    expect(l.match.line).toBe(42);
    expect(l.match.col).toBe(7);
  });

  it("html path with spaces split by wrap", () => {
    const buf = fakeBuffer(String.raw`wrote C:\Users\x y\My Docs\index.html now`, 16);
    const [l] = logicalLinks(buf, 2, findHtmlLinks);
    expect(l.match.path).toBe(String.raw`C:\Users\x y\My Docs\index.html`);
    expect(l.range.start).toEqual({ x: 7, y: 1 });
  });

  it("URL split over rows, trailing punctuation and quotes left out", () => {
    const buf = fakeBuffer("open `https://example.com/very/long/path?q=1`, then", 24);
    const [l] = logicalLinks(buf, 2, findWebUrls);
    expect(l.text).toBe("https://example.com/very/long/path?q=1");
    expect(l.range.start.y).toBe(1);
    expect(l.range.end.y).toBe(2);
    const [p] = logicalLinks(fakeBuffer("(see https://a.io/x_(y)).", 80), 1, findWebUrls);
    expect(p.text).toBe("https://a.io/x_(y)");
  });

  it("wide chars: offsets map to cells", () => {
    const buf = fakeBuffer("你好你好你 src/a.ts:1", 10);
    const [l] = paths(buf, 2);
    expect(l.text).toBe("src/a.ts:1");
    expect(l.range.start).toEqual({ x: 2, y: 2 });
    const wideLine = fakeBuffer("你好 a/b.ts", 40);
    const [w] = paths(wideLine, 1);
    expect(w.range.start).toEqual({ x: 6, y: 1 });
    expect(w.range.end).toEqual({ x: 11, y: 1 });
  });

  it("a link on another row of the logical line is not returned for rows it misses", () => {
    const buf = fakeBuffer("a/b.ts " + "x".repeat(60), 20);
    expect(paths(buf, 3)).toEqual([]);
    expect(paths(buf, 1)).toHaveLength(1);
  });

  it("caps the rows read", () => {
    const l = readLogical(fakeBuffer("x".repeat(1000), 10), 50);
    expect(l && l.text.length).toBeLessThan(10 * 60);
  });
});
