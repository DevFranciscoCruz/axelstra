import type { Widen } from "./i18n";
import type { ptW } from "./strings.welcome";
/** English texts of the empty main-area screen (no project, or no worktree chosen). */
export const enW: Widen<typeof ptW> = {
  title: "Welcome to Axelstra",
  flow: "Add a project, create a worktree and open an agent.",
  add: "Add project",
  pickTitle: "Pick a worktree",
  pickFlow: (project?: string) =>
    project
      ? `Open a worktree of ${project} or create a new one for a task.`
      : "Open a worktree or create a new one for a task.",
  newWorktree: "New worktree",
  detached: "(detached)",
  mainTag: "main",
  palette: "command palette",
  shortcuts: "shortcuts",
  needs: "To get started",
  needGit: { name: "git", why: "required, without it there are no worktrees" },
  needAgent: { name: "claude", why: "or another agent (for example codex)" },
  toolFound: "installed",
  toolMissing: "not found",
  needGh: { name: "gh", why: "only needed for PRs" },
} as const;
