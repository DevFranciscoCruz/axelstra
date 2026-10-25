import { moveRow } from "../hostLogic";

/** The origins of the Add project dialog, in the order of its left list. */
export const SOURCES = ["local", "clone", "github", "new", "remote", "discover"] as const;
export type Source = (typeof SOURCES)[number];
export const isSource = (v: unknown): v is Source =>
  typeof v === "string" && (SOURCES as readonly string[]).includes(v);

/** Next tab for a key on the vertical list (arrows wrap, Home/End jump); the same index when the key is not a navigation key. */
export const nextSource = (cur: Source, key: string): Source => {
  const i = SOURCES.indexOf(cur);
  const n = moveRow(i, SOURCES.length, key === "ArrowRight" ? "ArrowDown" : key === "ArrowLeft" ? "ArrowUp" : key);
  return SOURCES[n] ?? cur;
};

export type UrlProblem = "empty" | "space" | "dash" | "scheme";

/** A clone address the daemon accepts: https://, ssh:// or scp-like git@host:path. Never starts with «-» (an option). */
export const urlProblem = (raw: string): UrlProblem | null => {
  const u = raw.trim();
  if (!u) return "empty";
  if (/\s/.test(u)) return "space";
  if (u.startsWith("-")) return "dash";
  if (/^(?:https:\/\/|ssh:\/\/)[^/:@\s]+/i.test(u) || /^[\w.-]+@[\w.-]+:[^\s]+$/.test(u)) return null;
  return "scheme";
};

/** Last path segment of a repository address without «.git»: `https://github.com/o/r.git` -> `r`, `git@h:o/r` -> `r`. "" when none. */
export const repoNameFromUrl = (raw: string): string => {
  const u = raw
    .trim()
    .replace(/^[a-z][a-z0-9+.-]*:\/\//i, "")
    .replace(/[?#].*$/, "")
    .replace(/\/+$/, "")
    .replace(/\.git$/i, "");
  const seg =
    u
      .split(/[/:\\]/)
      .filter(Boolean)
      .pop() ?? "";
  return /^[\w][\w.-]*$/.test(seg) && !/^[.]+$/.test(seg) ? seg : "";
};

export type NameProblem = "empty" | "sep" | "dots" | "reserved" | "chars" | "long" | "trail";

const RESERVED = /^(?:con|prn|aux|nul|com[1-9]|lpt[1-9])(?:\..*)?$/i;

/** A folder name for a new project: one segment, no separators, no «..», no Windows reserved names or characters. */
export const nameProblem = (raw: string): NameProblem | null => {
  const n = raw.trim();
  if (!n) return "empty";
  if (/[\\/]/.test(n)) return "sep";
  if (/^\.+$/.test(n)) return "dots";
  if (n.length > 100) return "long";
  if (/[<>:"|?*]/.test(n) || [...n].some((c) => c.charCodeAt(0) < 32)) return "chars";
  if (/[. ]$/.test(n)) return "trail";
  if (RESERVED.test(n)) return "reserved";
  return null;
};

/** `name` inside `parent`, with the separator the parent already uses (backslash when it has one). */
export const joinPath = (parent: string, name: string): string => {
  const p = parent.trim();
  if (!p) return name;
  const sep = p.includes("\\") && !p.includes("/") ? "\\" : /^[A-Za-z]:$/.test(p) ? "\\" : "/";
  return p.endsWith("/") || p.endsWith("\\") ? p + name : p + sep + name;
};

/** a git depth the daemon accepts: empty (full) or a whole number from 1 to 1000000 */
export const depthProblem = (raw: string): boolean => {
  const d = raw.trim();
  return d !== "" && !(/^\d{1,7}$/.test(d) && Number(d) >= 1);
};

export type Repo = {
  name_with_owner: string;
  description?: string | null;
  is_private: boolean;
  url: string;
  pushed_at?: string | null;
};

/** repositories matching every word of `query` in the name or description (case-insensitive); all of them for an empty query */
export const filterRepos = <T extends Pick<Repo, "name_with_owner" | "description">>(
  repos: T[],
  query: string,
): T[] => {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  if (!words.length) return repos;
  return repos.filter((r) => {
    const hay = `${r.name_with_owner} ${r.description ?? ""}`.toLowerCase();
    return words.every((w) => hay.includes(w));
  });
};

/** keyboard on a list with a moving highlight: the new row, or `activate` for Enter (the row to open). */
export const listKey = (cur: number, n: number, key: string): { row: number; activate: boolean } => {
  if (key === "Enter") return { row: cur, activate: cur >= 0 && cur < n };
  return { row: ["ArrowDown", "ArrowUp", "Home", "End"].includes(key) ? moveRow(cur, n, key) : cur, activate: false };
};

/** a dropped path is a folder when it has no extension of a file: the webview cannot stat, so this only hints */
export const looksLikeFolder = (p: string): boolean =>
  !/\.[A-Za-z0-9]{1,6}$/.test(
    p
      .replace(/[\\/]+$/, "")
      .split(/[\\/]/)
      .pop() ?? "",
  );

// ---- remembered choices (per user, in the webview) ----
const TAB_KEY = "axelstra.addProject.tab";
const PARENT_KEY = "axelstra.addProject.cloneParent";

const read = (k: string): string => {
  try {
    return localStorage.getItem(k) ?? "";
  } catch {
    return "";
  }
};
const write = (k: string, v: string) => {
  try {
    localStorage.setItem(k, v);
  } catch {
    /* private window or blocked storage: the choice is simply not remembered */
  }
};

export const lastSource = (): Source => {
  const v = read(TAB_KEY);
  return isSource(v) ? v : "local";
};
export const rememberSource = (s: Source) => write(TAB_KEY, s);
export const lastParent = (): string => read(PARENT_KEY);
export const rememberParent = (p: string) => write(PARENT_KEY, p.trim());
