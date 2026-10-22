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
  if (/^[A-Za-z]:\//.test(slashed)) {
    if (!norm(slashed).startsWith(norm(worktree) + "/")) return { outside: path };
    return { rel: slashed.slice(worktree.replace(/\\/g, "/").replace(/\/$/, "").length + 1) };
  }
  return { rel: slashed.replace(/^\.\//, "") };
};
