import { type Widen, count, plural } from "./i18n";
import type { ptU } from "./strings.ui";

/** English texts that sit directly in small components. */
export const enU: Widen<typeof ptU> = {
  boundary: {
    whole: "Axelstra hit an error.",
    part: (name: string) => `This part (${name}) hit an error.`,
    terminalsKeep: "Terminals keep running.",
    copyDiag: "Copy diagnostics",
    reload: "Reload",
  },
  loading: {
    slow: "This is taking longer than usual.",
  },
  closeConfirm: {
    label: "Close window",
    title: "Close Axelstra?",
    none: "No sessions are running.",
    keepOn: (n: number) =>
      `${count(n, "session keeps", "sessions keep")} running in the background (and using resources) until you reopen the app.`,
    keepOff: (n: number) =>
      `${count(n, "session is closed", "sessions are closed")} and nothing keeps using resources. ${plural(n, "It comes", "They come")} back when you open the app again; agents resume the conversation.`,
    keepBackground: "Keep sessions and automations running in the background",
    dontAsk: "Don't ask again",
  },
  dispatcher: {
    title: "Dispatcher",
    note: "Each request made on main goes to the worktree of its topic. Here you see where each one went.",
    empty: "No requests dispatched yet. Ask the main agent for something.",
    waiting: "waiting for a reply",
    working: "working",
    idle: "idle",
    noAgent: "no agent",
    archived: "archived",
    openWorktree: "Open this worktree",
  },
  shortcuts: {
    title: "Keyboard shortcuts",
  },
  rightTabs: {
    label: "Right panel",
    files: "Files",
    changes: "Changes",
    prs: "PRs",
    hide: "Hide right panel (Ctrl+Shift+B)",
  },
  reports: {
    list: "Reports from this worktree's agents",
    open: "Open the report",
    status: { ok: "done", nota: "with notes", aberto: "no report" },
  },
  createWorktree: {
    title: "New worktree",
    project: "Project",
    name: "Name",
    namePh: "worktree name (also the branch name)",
    agent: "Agent",
    noAgent: "None (worktree only)",
    create: "Create worktree",
  },
  renameWorktree: {
    title: "Rename worktree",
    name: "Name",
    submit: "Rename",
  },
};
