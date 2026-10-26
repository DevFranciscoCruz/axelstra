/** the part of xterm's IBufferLine/IBufferCell the link code reads (kept small so tests can fake a buffer) */
export interface CellLike {
  getChars(): string;
  getWidth(): number;
}
export interface LineLike {
  readonly isWrapped: boolean;
  readonly length: number;
  getCell(x: number): CellLike | undefined;
}
export interface BufferLike {
  getLine(y: number): LineLike | undefined;
}

/** 1-based buffer coordinates, as xterm's link ranges use them */
export type Pos = { x: number; y: number };
export type Range = { start: Pos; end: Pos };

/** a logical line: the rows joined by soft wrap, with the buffer position of every UTF-16 unit of `text` */
export interface Logical {
  text: string;
  /** per unit of `text`: 1-based column and row of its cell, and the cell's width */
  at: { x: number; y: number; w: number }[];
}

/** most rows read above and below the asked row: a sane cap on the length of a logical line */
export const MAX_WRAP_ROWS = 24;

/**
 * The logical line holding row `y` (1-based): walks up while the row is wrapped (continues the row above) and down
 * while the next row is wrapped, so a path split by the terminal's soft wrap (also by a resize/reflow: `isWrapped` is
 * read live) is one string. Cells are read one by one: wide characters count once (their width-0 continuation cell is
 * skipped) and the offsets map back to the right cell. Empty cells at the end of a row that continues are dropped (the
 * padding a wide character leaves when it does not fit), real spaces stay; the end of the last row is trimmed.
 */
export function readLogical(buf: BufferLike, y: number): Logical | undefined {
  if (!buf.getLine(y - 1)) return undefined;
  let first = y;
  while (first > 1 && y - first < MAX_WRAP_ROWS && buf.getLine(first - 1)?.isWrapped) first--;
  let last = y;
  while (last - y < MAX_WRAP_ROWS && buf.getLine(last)?.isWrapped) last++;
  let text = "";
  const at: Logical["at"] = [];
  let spaces = 0; // trailing blank units of the row being read, kept pending
  for (let r = first; r <= last; r++) {
    const line = buf.getLine(r - 1);
    if (!line) break;
    const isLast = r === last;
    const mark = at.length;
    for (let x = 0; x < line.length; x++) {
      const cell = line.getCell(x);
      if (!cell) break;
      const w = cell.getWidth();
      if (w === 0) continue;
      const chars = cell.getChars();
      const piece = chars === "" ? " " : chars;
      for (let i = 0; i < piece.length; i++) at.push({ x: x + 1, y: r, w });
      text += piece;
      if (chars === "") spaces++;
      else spaces = 0;
    }
    if (isLast) {
      // trim the blanks at the end of the logical line
      const real = text.replace(/\s+$/, "").length;
      text = text.slice(0, real);
      at.length = real;
    } else if (spaces > 0) {
      // padding of a row that continues: empty cells only (a typed space is a real character)
      text = text.slice(0, text.length - spaces);
      at.length = Math.max(mark, at.length - spaces);
    }
    spaces = 0;
  }
  return { text, at };
}

/** the range of `text[start, end)` in buffer coordinates; it may span several rows */
export function rangeOf(l: Logical, start: number, end: number): Range {
  const a = l.at[start];
  const b = l.at[end - 1];
  return { start: { x: a.x, y: a.y }, end: { x: b.x + b.w - 1, y: b.y } };
}

export interface Match {
  /** offsets into the logical text, end exclusive */
  start: number;
  end: number;
}

export interface TermLink<M extends Match> {
  text: string;
  range: Range;
  match: M;
}

/**
 * The one place that turns "find links in a string" into links of the terminal: reads the logical line of row `y`,
 * runs `find` on all of it and returns the matches that touch row `y` with their (multi-row) ranges. Every link
 * provider of the terminal (html paths, URLs, editor paths) goes through it, so wrapped text behaves the same everywhere.
 */
export function logicalLinks<M extends Match>(buf: BufferLike, y: number, find: (text: string) => M[]): TermLink<M>[] {
  const l = readLogical(buf, y);
  if (!l || !l.text) return [];
  const out: TermLink<M>[] = [];
  for (const m of find(l.text)) {
    if (m.end <= m.start || m.end > l.at.length) continue;
    const range = rangeOf(l, m.start, m.end);
    if (range.start.y > y || range.end.y < y) continue;
    out.push({ text: l.text.slice(m.start, m.end), range, match: m });
  }
  return out;
}
