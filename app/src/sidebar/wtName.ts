export type WtNameProblem = "chars" | "dots" | "exists";

/**
 * Why a new worktree name cannot work, before the daemon is asked: git refuses these as branch names (the name is also the
 * branch), and a name that is already a worktree or branch of the project would only fail after the checkout started.
 * `taken` holds the branches and folder names of the project's worktrees.
 */
export function wtNameProblem(raw: string, taken: string[]): WtNameProblem | null {
  const name = raw.trim();
  if (!name) return null;
  if (name.includes("..") || name.includes("@{")) return "dots";
  // eslint-disable-next-line no-control-regex -- control characters are exactly what git refuses
  if (
    /[\s~^:?*[\\\x00-\x1f\x7f]/.test(name) ||
    /^[-./]/.test(name) ||
    /[/.]$/.test(name) ||
    name.endsWith(".lock") ||
    name === "@"
  )
    return "chars";
  const low = name.toLowerCase();
  if (taken.some((t) => t.toLowerCase() === low || t.toLowerCase() === `ax/${low}`)) return "exists";
  return null;
}
