import { type Widen, count } from "./i18n";
import type { ptL } from "./strings.logic";

/** English texts built by the logic modules (commands and cheat sheet, status dots, friendly errors, notifications, ...). */
export const enL: Widen<typeof ptL> = {
  groups: {
    geral: "General",
    vista: "View",
    separadores: "Tabs and terminals",
    worktree: "Worktrees and projects",
    ferramentas: "Tools",
    rato: "Mouse",
  },
  actionHint: "action",
  lang: { title: "Language", note: "Applies immediately, no restart. It is saved in the settings." },
  cmd: {
    palette: { label: "Command palette" },
    settings: { label: "Settings" },
    shortcuts: { label: "Keyboard shortcuts" },
    contextMenu: { label: "Context menu of the focused element" },
    confirm: { label: "Confirm the dialog, the card or the commit" },
    toggleSidebar: { label: "Show/hide projects" },
    toggleRight: { label: "Show/hide right panel" },
    panelFiles: { label: "Panel: files", hint: "panel" },
    panelChanges: { label: "Panel: changes", hint: "panel" },
    panelPulls: { label: "Panel: PRs and issues", hint: "panel" },
    docker: { label: "Docker", hint: "view" },
    kanban: { label: "Kanban", hint: "view" },
    notifications: { label: "Notifications", hint: "alert history" },
    theme: { label: "Change theme", hint: "system → dark → light" },
    language: { label: "Change language", hint: "Portuguese → English → Portuguese (Brazil)" },
    nextTab: { label: "Next / previous tab" },
    gotoTab: { label: "Go to tab n" },
    reopen: { label: "Reopen the last closed tab" },
    nextWaiting: { label: "Go to the next agent waiting for a reply" },
    rename: { label: "Rename the tab" },
    termFind: { label: "Search the terminal" },
    newAgent: { label: "New agent", hint: "agent" },
    newTerminal: { label: "New terminal", hint: "terminal" },
    split: { label: "Split terminal", hint: "view" },
    newWorktree: { label: "New worktree", hint: "worktree" },
    removeWorktree: { label: "Remove current worktree", hint: "worktree" },
    searchChats: { label: "Search conversations" },
    voice: { label: "Voice dictation" },
    orchestra: { label: "Orchestration: agents and workers", hint: "tool" },
    dispatcher: { label: "Dispatcher", hint: "requests sent to each worktree" },
    addRemote: { label: "Add remote project", hint: "SSH or WSL host" },
    automations: { label: "Automations", hint: "scheduling" },
    computer: { label: "Screen control by agents", hint: "permission" },
    restartDaemon: { label: "Restart the daemon", hint: "closes the terminals; agents resume" },
    tabRename: { label: "Rename a tab", keys: "Double-click the tab" },
    tabClose: { label: "Close a tab", keys: "Middle-click the tab" },
  },

  dot: {
    working: "Working",
    idle: "Idle",
    waiting: "Waiting for a reply",
    done: "Finished",
    attention: "Stopped: needs attention",
    queued: "Queued: the plan has not reached this task yet",
    running: "Terminal running",
    none: "",
  },
  session: (id: number) => `Session ${id}`,
  defaultAgent: "The agent",
  tabRenameFailed: "Could not save the tab name: ",
  unsavedFile: "This file has unsaved changes.",
  editorTitle: "Editor",
  pickProject: "Choose the project folder",
  unknownError: "Unknown error",

  friendly: {
    notFound: "Program not found. Check that it is installed and on the PATH (git, claude, codex or gh).",
    exists: "A branch or worktree with that name already exists. Choose another name.",
    notRepo: "This folder is not a git repository.",
    dirty: "There are unsaved changes. Commit or discard them first.",
    remote: "No access to the remote. Check your internet connection and your git/gh login.",
    rejected: "The remote refused the push because it has new commits. Pull the remote changes before pushing.",
    workerLimit: "Limit of simultaneous agents reached. Wait for one to finish or raise it with `axelstra limit`.",
    noDaemon: "No connection to the daemon. It will try again; if it persists, restart Axelstra.",
    daemonSilent: "The daemon did not answer in time. If it persists, restart Axelstra.",
    oldDaemon: "The running daemon is outdated. Restart Axelstra to update it.",
    dockerDown: "Docker is not running. Open Docker Desktop (or start the service) and try again.",
    dockerGone: "Docker no longer has that resource: it was deleted in the meantime. Refresh the list.",
    portBusy: "The port is already in use by another program. Stop that program or choose another port.",
    ghAuth: "gh is not signed in. Run `gh auth login` in a terminal.",
    conflicts: "There are merge conflicts. Resolve the conflicting files and commit.",
    nothingToCommit: "There are no changes to commit.",
    badRef: "The given branch or commit does not exist in this repository.",
    rateLimit: "GitHub rate-limited the requests. Wait a few minutes and try again.",
  },

  editor: {
    binary: "Binary file: it cannot be opened in the editor.",
    notUtf8: "The file is not UTF-8 text: it cannot be opened in the editor.",
    tooLarge: "File too large for the editor.",
  },

  ago: {
    now: "now",
    min: (n: number) => `${n} min ago`,
    hour: (n: number) => `${n} h ago`,
    day: (n: number) => `${n} d ago`,
  },
  usage: {
    none: "No agent used tokens in these worktrees",
    head: (spent: string, cache: string) => `Agent tokens (Claude and Codex): ${spent} spent · ${cache} of cache read`,
    row: (name: string, spent: string, input: string, output: string, write: string, read: string) =>
      `${name}: ${spent} (input ${input} · output ${output} · cache ${write}/${read})`,
  },

  agent: {
    notInstalled: (name: string) => `${name} (not installed)`,
    unknownInList: (id: string) => `${id} (unknown)`,
    unknown: (id: string) => `Unknown agent: ${id}`,
    notFound: (name: string, install: string) => `${name} not found: install it with ${install}`,
  },

  notify: {
    done: {
      head: (who: string, wt: string) => `${who} finished · ${wt}`,
      body: (who: string, wt: string) => `${who} finished the task in worktree "${wt}". Open it to review the result.`,
    },
    waiting: {
      head: (who: string, wt: string) => `${who} needs you · ${wt}`,
      body: (who: string, wt: string) =>
        `${who} asked a question in worktree "${wt}" and needs your answer to continue.`,
    },
    stopped: {
      head: (who: string, wt: string) => `${who} stopped · ${wt}`,
      body: (who: string, wt: string) =>
        `${who} stopped in worktree "${wt}" without finishing. Open it to see what happened.`,
    },
    ship: {
      merged: (pr: string, wt: string) => `Merged: ${pr} (${wt})`,
      ciFail: (pr: string, wt: string) => `CI failing: ${pr} (${wt})`,
      ciPass: (pr: string, wt: string) => `CI passing: ${pr} (${wt})`,
    },
    janitor: (branch: string, into: string) => `worktree ${branch} removed (merged into ${into})`,
  },
  restore: {
    restored: (n: number) => count(n, "session restored", "sessions restored"),
    skipped: (n: number) => count(n, "empty conversation skipped", "empty conversations skipped"),
  },

  voice: {
    denied: "Microphone access was denied. Allow it in the Windows privacy settings.",
    missing: "No microphone was found. Plug one in and try again.",
    busy: "The microphone is being used by another application.",
    failed: "Could not use the microphone. Try again.",
    noModel: "The dictation model is not available. Download it again.",
    transcribeFailed: "Transcription failed. Try dictating again.",
    downloadFailed: "Could not download the dictation model. Check your connection and try again.",
  },
};
