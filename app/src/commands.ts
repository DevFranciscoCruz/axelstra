import { L, type ptL } from "./strings.logic";
/**
 * The one list of commands. The palette offers every command that has a handler right now; the cheat sheet (?) lists
 * every command that has keys. A shortcut added here shows up in both.
 */
export type CommandGroup = "geral" | "vista" | "separadores" | "worktree" | "ferramentas" | "rato";

export type Command = {
  id: string;
  label: string;
  /** shown in the cheat sheet and as the palette hint; written with Ctrl (K() renames it on a Mac) */
  keys?: string;
  group: CommandGroup;
  /** palette hint when there are no keys */
  hint?: string;
  /** false: listed in the cheat sheet only (the key is handled where the thing it acts on lives) */
  palette?: boolean;
};

/** the words come from the dictionary at read time, so the palette and the cheat sheet follow the language */
export const GROUP_LABEL: Record<CommandGroup, string> = L.groups;

type Raw = { id: keyof typeof ptL.cmd; keys?: string; group: CommandGroup; palette?: boolean };
const RAW: Raw[] = [
  { id: "palette", keys: "Ctrl+K", group: "geral", palette: false },
  { id: "settings", keys: "Ctrl+,", group: "geral" },
  { id: "shortcuts", keys: "?", group: "geral" },
  { id: "contextMenu", keys: "Shift+F10", group: "geral", palette: false },
  { id: "confirm", keys: "Ctrl+Enter", group: "geral", palette: false },

  { id: "toggleSidebar", keys: "Ctrl+B", group: "vista" },
  { id: "toggleRight", keys: "Ctrl+Shift+B", group: "vista" },
  { id: "panelFiles", group: "vista" },
  { id: "panelChanges", group: "vista" },
  { id: "panelPulls", group: "vista" },
  { id: "docker", group: "vista" },
  { id: "kanban", group: "vista" },
  { id: "notifications", group: "vista" },
  { id: "theme", group: "vista" },
  { id: "language", group: "vista" },

  { id: "nextTab", keys: "Ctrl+Tab / Ctrl+Shift+Tab", group: "separadores", palette: false },
  { id: "gotoTab", keys: "Ctrl+1 … Ctrl+9", group: "separadores", palette: false },
  { id: "reopen", keys: "Ctrl+Shift+T", group: "separadores" },
  { id: "nextWaiting", keys: "Ctrl+J", group: "separadores" },
  { id: "rename", keys: "F2", group: "separadores" },
  { id: "termFind", keys: "Ctrl+F", group: "separadores", palette: false },
  { id: "newAgent", group: "separadores" },
  { id: "newTerminal", group: "separadores" },
  { id: "split", group: "separadores" },

  { id: "newWorktree", group: "worktree" },
  { id: "removeWorktree", group: "worktree" },

  { id: "searchChats", keys: "Ctrl+Shift+F", group: "ferramentas" },
  { id: "voice", keys: "Ctrl+Shift+M", group: "ferramentas" },
  { id: "orchestra", group: "ferramentas" },
  { id: "dispatcher", group: "ferramentas" },
  { id: "addRemote", group: "worktree" },
  { id: "automations", group: "ferramentas" },
  { id: "computer", group: "ferramentas" },
  { id: "restartDaemon", group: "ferramentas" },

  { id: "tabRename", group: "rato", palette: false },
  { id: "tabClose", group: "rato", palette: false },
];

type CmdText = { label: string; hint?: string; keys?: string };
export const COMMANDS: Command[] = RAW.map((r) => {
  const text = (): CmdText => L.cmd[r.id];
  return Object.defineProperties({ id: r.id, group: r.group, palette: r.palette } as Command, {
    label: { get: () => text().label, enumerable: true },
    hint: { get: () => text().hint, enumerable: true },
    keys: { get: () => r.keys ?? text().keys, enumerable: true },
  });
});

const byId = new Map(COMMANDS.map((c) => [c.id, c]));
export const commandById = (id: string): Command | undefined => byId.get(id);

/** a runtime hook for a command: `label` replaces the registry one when it changes with state ("Mostrar" / "Ocultar") */
export type Handler = { run: () => void; label?: string };

/** [keys, what] rows of the cheat sheet, grouped */
export const cheatSheet = (): { group: CommandGroup; title: string; rows: [string, string][] }[] => {
  const order = Object.keys(GROUP_LABEL) as CommandGroup[];
  return order
    .map((g) => ({
      group: g,
      title: GROUP_LABEL[g],
      rows: COMMANDS.filter((c) => c.group === g && c.keys).map((c) => [c.keys!, c.label] as [string, string]),
    }))
    .filter((g) => g.rows.length);
};

/** palette entries for the commands that have a handler right now, in registry order */
export const paletteCommands = (
  handlers: Partial<Record<string, Handler | undefined>>,
  fmtKeys: (k: string) => string = (k) => k,
) =>
  COMMANDS.flatMap((c) => {
    const h = handlers[c.id];
    if (!h || c.palette === false) return [];
    return [
      { id: c.id, label: h.label ?? c.label, hint: c.keys ? fmtKeys(c.keys) : (c.hint ?? L.actionHint), run: h.run },
    ];
  });
