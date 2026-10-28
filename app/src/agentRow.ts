import type { SessionInfo } from "./api";
import { norm } from "./store";
import { L } from "./strings.logic";

/** Last folder of a path: the name a worktree goes by in the list. */
export const folderName = (path?: string | null) =>
  (path ?? "").replace(/\\/g, "/").replace(/\/+$/, "").split("/").pop() ?? "";

/** What an agent row in the sidebar shows besides the state dot: the name, the agent kind as a small tag and, when
 *  the agent runs in another worktree than the one it is listed under (a worker), that worktree's name. */
export const agentRowInfo = (
  s: Pick<SessionInfo, "id" | "label" | "meta">,
  listedUnder: string,
  shownName?: string,
) => {
  const name = (shownName ?? s.label ?? "").trim() || L.session(s.id);
  const kind = s.meta?.agent ?? "";
  const own = s.meta?.worktree;
  const worktree = own && norm(own) !== norm(listedUnder) ? folderName(own) : "";
  return { name, kind, worktree, hasReport: !!s.meta?.note, lead: s.meta?.lead === true };
};

/** Accessible name of a whole row, read in one go by a screen reader. */
export const agentRowLabel = (i: ReturnType<typeof agentRowInfo>, state: string) =>
  [i.name, i.kind, i.worktree && L.inWorktree(i.worktree), state].filter(Boolean).join(", ");

/** Tooltip of the "same files" badge on a worktree row; null when nothing overlaps. */
export const overlapTitle = (others: string[] | undefined) => {
  const names = (others ?? []).map(folderName).filter(Boolean);
  return names.length ? L.sameFiles(names.join(", ")) : null;
};

/** The overlaps of one worktree, looked up by normalised path (the daemon's keys may use another slash style). */
export const overlapsOf = (map: Record<string, string[]> | null | undefined, path: string) => {
  if (!map) return undefined;
  const want = norm(path);
  for (const [k, v] of Object.entries(map)) if (norm(k) === want) return v;
  return undefined;
};
