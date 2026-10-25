/** A `.html`/`.htm` file path found in a line of terminal output. */
export type HtmlLink = {
  /** the path as printed (quotes and the `:line:col` suffix removed) */
  path: string;
  line?: number;
  col?: number;
  /** 0-based range in the line of the text to underline: the path plus its `:line:col`, never the quotes */
  start: number;
  end: number;
};

// A path name character: no whitespace, quote, shell or bracket punctuation, no separator. Brackets, commas and
// semicolons stay out so that "(a.html)", "[a.html]" and "a.html," lose their wrapper without any trimming.
const NAME = String.raw`[^\s"'` + "`" + String.raw`<>|*?:()\[\]{},;\\/]`;
const EXT = String.raw`\.html?`;
const LINECOL = String.raw`(?::(\d+)(?::(\d+))?)?`;
// the end of a match: not a longer extension (.htmlx), not a longer name (.html.bak, .html-old), not a directory (.html/)
const END = String.raw`(?![\w\-/\\]|\.\w)`;

// 1. quoted: any text between matching quotes that ends in .html/.htm (spaces allowed: the quotes bound it)
const QUOTED = String.raw`(["'` + "`" + String.raw`])([^"'` + "`" + String.raw`\n]*?${EXT})${LINECOL}\1`;
// 2. drive-absolute, spaces allowed: it starts at "C:\" and ends at the extension, so the line text bounds it. A space
// may not be followed by a separator ("x \y" is two things) and a segment may not run past an ".html" ("a.html and C/b.html").
const SPACE_SEG = String.raw`(?:(?!${EXT}(?![\w]))(?: (?![\\/])|[^\s"'` + "`" + String.raw`<>|*?:\\/]))`;
const DRIVE = String.raw`([A-Za-z]:[\\/](?:${SPACE_SEG}+[\\/])*${SPACE_SEG}+?${EXT})${LINECOL}${END}`;
// 3. everything else, no spaces: POSIX absolute, ./ and ../ relative, a/b.html, index.html
const PLAIN = String.raw`((?:\.{1,2}[\\/]|[\\/])?(?:${NAME}+[\\/])*${NAME}+?${EXT})${LINECOL}${END}`;

const RE = new RegExp(`${QUOTED}|${DRIVE}|${PLAIN}`, "gi");
const URL_RE = /[A-Za-z][A-Za-z0-9+.-]*:\/\/\S*/g;

/** characters that make a match start in the middle of a longer token (a/b.html found inside x.a/b.html, a URL tail, ...) */
const TOKEN_CHAR = /[\w.\-/\\~:]/;

/**
 * Finds the .html/.htm file paths in one terminal line.
 *
 * Spaces: a path with spaces is linked only when it is quoted ("a b/c.html", 'a b/c.html', `a b/c.html`) or when it
 * is a Windows drive path (C:\Users\x y\index.html), whose start (the drive) and end (the extension) the line bounds.
 * Anything else stops at the first space. Conservative on purpose: a wrong link is worse than a missing one.
 * Not matched: URLs (http(s) links belong to the web-links handling), .htmlx, .html.bak, directories named x.html/.
 * A trailing `.`, `,`, `;`, `)` or `]` is never part of a match: a path ends at its extension or its `:line:col`.
 */
export function findHtmlLinks(line: string): HtmlLink[] {
  const urls: [number, number][] = [];
  for (const m of line.matchAll(URL_RE)) urls.push([m.index, m.index + m[0].length]);
  const out: HtmlLink[] = [];
  RE.lastIndex = 0;
  for (let m = RE.exec(line); m; m = RE.exec(line)) {
    const quoted = m[1] !== undefined;
    const path = m[2] ?? m[5] ?? m[8];
    const l = m[3] ?? m[6] ?? m[9];
    const c = m[4] ?? m[7] ?? m[10];
    const suffix = (l !== undefined ? `:${l}` : "") + (c !== undefined ? `:${c}` : "");
    const lead = quoted ? 1 : 0; // the opening quote
    const start = m.index + lead;
    const end = start + path.length + suffix.length;
    const inUrl = urls.some(([a, b]) => start < b && end > a);
    const midToken = !quoted && start > 0 && TOKEN_CHAR.test(line[start - 1]);
    if (inUrl || midToken) {
      RE.lastIndex = m.index + 1; // a later start inside this text may still be a clean match
      continue;
    }
    out.push({
      path,
      ...(l !== undefined && { line: Number(l) }),
      ...(c !== undefined && { col: Number(c) }),
      start,
      end,
    });
  }
  return out;
}
