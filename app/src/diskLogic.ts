import type { DiskStatus, DiskWorktree } from "./api";
import { fmtFixed, fmtNumber } from "./i18n";

/** unintegrated work idle longer than this is called out in the disk panel */
export const STALE_MS = 24 * 3600_000;

/** A size given in MB, the way the UI language writes it: "512 MB", "9,1 GB", "12 GB" (one decimal below 10 GB). */
export const fmtMb = (mb: number): string => {
  const v = Math.max(0, mb);
  if (v < 1024) return `${fmtNumber(Math.round(v))} MB`;
  const gb = v / 1024;
  return gb < 10 ? `${fmtFixed(gb, 1)} GB` : `${fmtNumber(Math.round(gb))} GB`;
};

/** branch name without the `ax/` Axelstra puts in front, else the folder name */
export const wtName = (w: Pick<DiskWorktree, "branch" | "path">): string =>
  w.branch
    ? w.branch.replace(/^ax\//, "")
    : (w.path
        .split(/[\\/]+/)
        .filter(Boolean)
        .pop() ?? w.path);

/** no session, no build, no flow uses it (the daemon's `active`) */
export const inactive = (w: DiskWorktree): boolean => !w.active && w.sessions.length === 0;

/** Work that is not in the default branch (`ahead` commits, not integrated) on a worktree nobody used for more than a day.
 *  A worktree whose last use is unknown is not called out: the warning must not cry wolf. */
export const isStale = (w: DiskWorktree, now: number): boolean =>
  !w.integrated && w.ahead > 0 && inactive(w) && w.last_used_ms != null && now - w.last_used_ms > STALE_MS;

/** the stale worktrees, biggest first */
export const staleList = (s: DiskStatus | null, now: number): DiskWorktree[] =>
  (s?.worktrees ?? []).filter((w) => isStale(w, now)).sort((a, b) => b.total_mb - a.total_mb);

/** the worktrees by size, biggest first */
export const bySize = (s: DiskStatus | null): DiskWorktree[] =>
  [...(s?.worktrees ?? [])].sort((a, b) => b.total_mb - a.total_mb);

/** MB the global "Limpar builds" may free (inactive worktrees only; the daemon answers 0 for an active one) */
export const reclaimableTotal = (s: DiskStatus | null): number =>
  (s?.worktrees ?? []).filter(inactive).reduce((n, w) => n + w.reclaimable_mb, 0);

/** seconds since the worktree was last used, for `ago()` (0 when unknown) */
export const idleSecs = (w: DiskWorktree, now: number): number =>
  w.last_used_ms == null ? 0 : Math.max(0, Math.floor((now - w.last_used_ms) / 1000));

/** A whole number typed in a settings field, pulled into `min..max`; null when it is not a number at all. */
export const clampInt = (raw: string, min: number, max: number): number | null => {
  const n = Number(raw.trim());
  if (raw.trim() === "" || !Number.isFinite(n)) return null;
  return Math.min(max, Math.max(min, Math.round(n)));
};
