import type { RemoveMode, RemovePreview, RemoveResult } from "./api";
import { P } from "./strings.panels";
import { S } from "./strings";

const T = () => P.removeProject;

export type Preview = RemovePreview | "error" | null;

/** a worktree's display name: its branch, else the last segment of its folder */
export const wtName = (w: RemovePreview["worktrees"][number]) =>
  w.branch ||
  w.path
    .replace(/[\\/]+$/, "")
    .split(/[\\/]/)
    .pop() ||
  w.path;

/** the worktrees whose work "Remover tudo" would lose: uncommitted changes or commits not in the default branch */
export const riskLines = (p: Preview): string[] => {
  if (!p || p === "error") return [];
  return p.worktrees
    .filter((w) => w.dirty || w.unmerged)
    .map((w) =>
      T().riskItem(wtName(w), [...(w.dirty ? [T().riskDirty] : []), ...(w.unmerged ? [T().riskUnmerged] : [])]),
    );
};

/** what the chosen mode will do, one line each */
export const effectLines = (mode: RemoveMode, p: Preview): string[] => {
  if (mode === "list") return [...T().listEffects];
  if (!p || p === "error") return [];
  const out: string[] = [];
  if (p.sessions + p.terminals > 0) out.push(T().allSessions(p.sessions, p.terminals));
  if (p.worktrees.length > 0) out.push(T().allWorktrees(p.worktrees.length));
  if (out.length === 0) out.push(T().allNothing);
  out.push(T().allKept);
  return out;
};

/** "Remover tudo" with unsaved work asks for the project name; only "list" is open while the preview is missing or failed */
export const needsTyped = (mode: RemoveMode, p: Preview) => mode === "all" && riskLines(p).length > 0;
export const canChoose = (mode: RemoveMode, p: Preview) => mode === "list" || (p !== null && p !== "error");

/** the notice after a removal: the plain one for "list", the counts for "all" */
export const doneNotice = (name: string, mode: RemoveMode, r: RemoveResult | undefined) =>
  mode === "all" && r
    ? T().doneAll(
        name,
        r.sessions_closed ?? 0,
        r.worktrees_removed ?? 0,
        r.branches_deleted ?? 0,
        r.worktrees_pending ?? 0,
      )
    : S.projectRemoved(name);
