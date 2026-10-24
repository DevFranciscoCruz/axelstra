import { folderName } from "./agentRow";
import { norm } from "./paths";
import { P } from "./strings.panels";

const T = P.sidebar.rebase;

/** What `dispatch.overlap_files` says about one other worktree: the files both edit. */
export type OverlapEntry = { with: string; files: string[] };
export type OverlapFiles = Record<string, OverlapEntry[]>;

/** Answer of `worktree.rebase`: moved, or stopped by a conflict (the daemon already put the branch back). */
export type RebaseResult =
  | { ok: true; from: string; to: string; base?: string }
  | { ok: false; conflicted: string[]; restored: boolean; base?: string };

/** Files shown in the tooltip for each other worktree before the rest is summed up. */
const SHOWN = 4;

/** The overlaps of one worktree, looked up by normalised path (the daemon's keys may use another slash style). */
export const overlapEntries = (map: OverlapFiles | null | undefined, path: string): OverlapEntry[] => {
  if (!map) return [];
  const want = norm(path);
  for (const [k, v] of Object.entries(map)) if (norm(k) === want) return v;
  return [];
};

/** Distinct files that some other worktree also edits: the number on the badge. */
export const overlapCount = (entries: OverlapEntry[]) => new Set(entries.flatMap((e) => e.files)).size;

/** Tooltip of the badge: each other worktree with the files shared with it; null when nothing overlaps. */
export const overlapTitle = (entries: OverlapEntry[]) => {
  const lines = entries
    .filter((e) => e.files.length > 0)
    .map((e) => T.overlapWith(folderName(e.with), e.files.slice(0, SHOWN), Math.max(0, e.files.length - SHOWN)));
  return lines.length ? T.overlapTip(lines) : null;
};

/** The toast for an answer of `worktree.rebase`: kind "info" when it moved, "warn" on a conflict. */
export const rebaseOutcome = (name: string, base: string, r: RebaseResult) => {
  if (r.ok) return { kind: "info" as const, text: r.from === r.to ? T.upToDate(name, base) : T.done(name, base) };
  return { kind: "warn" as const, text: T.conflict(name, r.conflicted, r.restored) };
};

/** Refusal the app can tell before calling the daemon (it checks again): a busy or dirty worktree, as a message. */
export const rebaseBlocker = (name: string, st: { dirty?: number } | undefined, live: number) =>
  live > 0 ? T.busy(name) : st?.dirty ? T.dirty(name) : null;
