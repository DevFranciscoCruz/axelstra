/** file paths in terminal output: absolute Windows paths or relative ones with an extension, optionally followed by :line[:col] */
export const PATH_RE = /(?:[A-Za-z]:[\\/][^\s"'`<>|*?()]+|(?:\.{1,2}[\\/])?[\w@.-]+(?:[\\/][\w@.-]+)*\.[A-Za-z][\w]{0,7})(?::\d+(?::\d+)?)?/g;

export type PathLink = { text: string; start: number; path: string; line?: number; col?: number };

/** splits "src/a.ts:12:3" into the path and its position, dropping trailing punctuation */
export const parsePathLink = (raw: string): { path: string; line?: number; col?: number } => {
  const m = /:(\d+)(?::(\d+))?$/.exec(raw);
  const path = (m ? raw.slice(0, m.index) : raw).replace(/[.,;:]+$/, "");
  return { path, line: m ? Number(m[1]) : undefined, col: m?.[2] ? Number(m[2]) : undefined };
};

/** the clickable paths of one terminal line; `start` is the 0-based column */
export const findPathLinks = (line: string): PathLink[] => {
  const out: PathLink[] = [];
  for (const m of line.matchAll(PATH_RE)) {
    const at = m.index ?? 0;
    // a match that starts mid-token (inside a URL, say) is not a path
    if (at > 0 && /[\w:/@.-]/.test(line[at - 1])) continue;
    const text = m[0].replace(/[.,;:)]+$/, "");
    out.push({ text, start: at, ...parsePathLink(text) });
  }
  return out;
};

const norm = (p: string) => p.replace(/\\/g, "/").replace(/\/$/, "").toLowerCase();

/** where a clicked path points relative to the selected worktree: inside it (`rel`) or somewhere else (`outside`) */
export const resolvePathLink = (path: string, worktree: string): { rel: string } | { outside: string } => {
  const slashed = path.replace(/\\/g, "/");
  // a relative path that climbs out of the worktree is outside too
  if (!/^[A-Za-z]:\//.test(slashed) && slashed.split("/").includes("..")) return { outside: path };
  if (/^[A-Za-z]:\//.test(slashed)) {
    if (!norm(slashed).startsWith(norm(worktree) + "/")) return { outside: path };
    return { rel: slashed.slice(worktree.replace(/\\/g, "/").replace(/\/$/, "").length + 1) };
  }
  return { rel: slashed.replace(/^\.\//, "") };
};

/** the URL when it is safe to open from terminal output (http/https only), otherwise null: mailto:, tel:, file: and friends are refused */
export const safeLinkUrl = (raw: string): string | null => {
  try {
    const u = new URL(raw.trim());
    return u.protocol === "http:" || u.protocol === "https:" ? u.href : null;
  } catch {
    return null;
  }
};

/** the http(s) URLs of a (logical) line as `{start, end}` offsets, end exclusive; trailing punctuation and an unbalanced `)` are left out */
export const findWebUrls = (line: string): { start: number; end: number }[] => {
  const out: { start: number; end: number }[] = [];
  for (const m of line.matchAll(/https?:\/\/[^\s"'`<>]+/gi)) {
    let u = m[0];
    for (;;) {
      const t = u.replace(/[.,;:!?'"\]}]+$/, "");
      const open = (t.match(/\(/g) ?? []).length;
      const close = (t.match(/\)/g) ?? []).length;
      const next = t.endsWith(")") && close > open ? t.slice(0, -1) : t;
      if (next === u) break;
      u = next;
    }
    if (/^https?:\/\/.+/i.test(u)) out.push({ start: m.index ?? 0, end: (m.index ?? 0) + u.length });
  }
  return out;
};
