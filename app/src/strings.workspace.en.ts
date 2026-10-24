import { type Widen, count, plural } from "./i18n";
import type { ptWS } from "./strings.workspace";

/** English texts of the workspace export / import section of Settings. */
export const enWS: Widen<typeof ptWS> = {
  title: "Workspace configuration",
  note: "Saves projects, automations, settings and Kanban cards to a file, or brings them in from another machine. Never includes tokens, sessions or logs.",
  export: "Export…",
  import: "Import…",
  exportPick: "Save the configuration",
  importPick: "Choose the configuration file",
  mapPick: "Choose this project's folder on this machine",
  exported: (path: string) => `Configuration saved to ${path}`,
  noSupport: "This daemon cannot export or import the configuration yet. Update and restart Axelstra.",
  dialogTitle: "Import configuration",
  fileLine: (file: string, at: string) => (at ? `${file} · exported on ${at}` : file),
  kinds: { projects: "Projects", automations: "Automations", settings: "Settings", cards: "Cards" },
  nothing: "nothing",
  nNew: (n: number) => count(n, "new", "new"),
  nPresent: (n: number) => `${n} already identical`,
  nConflict: (n: number) => count(n, "different", "different"),
  nSkipped: (n: number) => count(n, "skipped", "skipped"),
  localOnly: (n: number) => count(n, "only here", "only here"),
  reasons: {
    missing: "the folder does not exist on this machine",
    forbidden: "the folder cannot be a project",
    duplicate: "another entry in the file uses the same folder",
    "project skipped": "the project was skipped",
    unsafe: "the .kanban folder is a symbolic link",
  } as Record<string, string>,
  reasonOther: "cannot be imported",
  moreSkipped: (n: number) => `and ${count(n, "more skipped", "more skipped")}`,
  mapButton: "Choose folder…",
  mapped: "Mapped folders",
  mapClear: "Clear mappings",
  risky: (n: number) =>
    `${count(n, "card runs", "cards run")} with permissions off or with an acceptance command. Read ${plural(n, "it", "them")} before you run ${plural(n, "it", "them")}.`,
  mode: "Mode",
  modeMerge: "Merge",
  modeMergeHint: "Adds what is missing and keeps what already exists.",
  modeReplace: "Replace",
  modeReplaceHint: "Swaps your projects, automations and settings for the ones in the file.",
  conflicts: "When different",
  conflictsKeep: "Keep mine",
  conflictsOverwrite: "Use the file's",
  replaceWarn: (n: number) =>
    `Replace removes from the list ${count(n, "project that is not", "projects that are not")} in the file. The previous state files are kept as .1. Cards are only replaced if you choose “Use the file's”.`,
  replaceWord: "replace",
  replaceType: (w: string) => `Type “${w}” to confirm`,
  cancel: "Cancel",
  confirm: "Import",
  importing: "Importing…",
  done: (o: {
    projects_added: number;
    automations_added: number;
    automations_updated: number;
    settings_set: number;
    cards_written: number;
    cards_kept: number;
  }) =>
    [
      count(o.projects_added, "new project", "new projects"),
      count(o.automations_added + o.automations_updated, "automation", "automations"),
      count(o.settings_set, "setting", "settings"),
      `${count(o.cards_written, "card written", "cards written")}${o.cards_kept ? `, ${count(o.cards_kept, "kept", "kept")}` : ""}`,
    ].join(" · "),
  doneTitle: "Configuration imported: ",
  restartHint: " Restart Axelstra to apply the settings.",
};
