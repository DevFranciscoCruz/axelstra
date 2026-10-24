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

export const GROUP_LABEL: Record<CommandGroup, string> = {
  geral: "Geral",
  vista: "Vista",
  separadores: "Separadores e terminais",
  worktree: "Worktrees e projetos",
  ferramentas: "Ferramentas",
  rato: "Rato",
};

export const COMMANDS: Command[] = [
  { id: "palette", label: "Paleta de comandos", keys: "Ctrl+K", group: "geral", palette: false },
  { id: "settings", label: "Definições", keys: "Ctrl+,", group: "geral" },
  { id: "shortcuts", label: "Atalhos de teclado", keys: "?", group: "geral" },
  { id: "contextMenu", label: "Menu de contexto do elemento focado", keys: "Shift+F10", group: "geral", palette: false },
  { id: "confirm", label: "Confirmar o diálogo, o cartão ou o commit", keys: "Ctrl+Enter", group: "geral", palette: false },

  { id: "toggleSidebar", label: "Mostrar/ocultar projetos", keys: "Ctrl+B", group: "vista" },
  { id: "toggleRight", label: "Mostrar/ocultar painel direito", keys: "Ctrl+Shift+B", group: "vista" },
  { id: "panelFiles", label: "Painel: ficheiros", hint: "painel", group: "vista" },
  { id: "panelChanges", label: "Painel: alterações", hint: "painel", group: "vista" },
  { id: "panelPulls", label: "Painel: PRs e problemas", hint: "painel", group: "vista" },
  { id: "docker", label: "Docker", hint: "vista", group: "vista" },
  { id: "kanban", label: "Kanban", hint: "vista", group: "vista" },
  { id: "notifications", label: "Notificações", hint: "histórico de avisos", group: "vista" },
  { id: "theme", label: "Mudar o tema", hint: "sistema → escuro → claro", group: "vista" },

  { id: "nextTab", label: "Separador seguinte / anterior", keys: "Ctrl+Tab / Ctrl+Shift+Tab", group: "separadores", palette: false },
  { id: "gotoTab", label: "Ir para o separador n", keys: "Ctrl+1 … Ctrl+9", group: "separadores", palette: false },
  { id: "reopen", label: "Reabrir o último separador fechado", keys: "Ctrl+Shift+T", group: "separadores" },
  { id: "nextWaiting", label: "Ir para o próximo agente à espera de resposta", keys: "Ctrl+J", group: "separadores" },
  { id: "rename", label: "Mudar o nome do separador", keys: "F2", group: "separadores" },
  { id: "termFind", label: "Procurar no terminal", keys: "Ctrl+F", group: "separadores", palette: false },
  { id: "newAgent", label: "Novo agente", group: "separadores", hint: "agente" },
  { id: "newTerminal", label: "Novo terminal", group: "separadores", hint: "terminal" },
  { id: "split", label: "Dividir terminal", group: "separadores", hint: "vista" },

  { id: "newWorktree", label: "Novo worktree", hint: "worktree", group: "worktree" },
  { id: "removeWorktree", label: "Remover worktree atual", hint: "worktree", group: "worktree" },

  { id: "searchChats", label: "Pesquisar conversas", keys: "Ctrl+Shift+F", group: "ferramentas" },
  { id: "voice", label: "Ditado por voz", keys: "Ctrl+Shift+M", group: "ferramentas" },
  { id: "orchestra", label: "Orquestração: agentes e workers", hint: "ferramenta", group: "ferramentas" },
  { id: "dispatcher", label: "Despachante", hint: "pedidos enviados a cada worktree", group: "ferramentas" },
  { id: "automations", label: "Automatizações", hint: "agendamento", group: "ferramentas" },
  { id: "computer", label: "Controlo do ecrã pelos agentes", hint: "permissão", group: "ferramentas" },
  { id: "restartDaemon", label: "Reiniciar o daemon", hint: "fecha os terminais; os agentes retomam", group: "ferramentas" },

  { id: "tabRename", label: "Mudar o nome de um separador", keys: "Duplo clique no separador", group: "rato", palette: false },
  { id: "tabClose", label: "Fechar um separador", keys: "Clique do meio no separador", group: "rato", palette: false },
];

const byId = new Map(COMMANDS.map((c) => [c.id, c]));
export const commandById = (id: string): Command | undefined => byId.get(id);

/** a runtime hook for a command: `label` replaces the registry one when it changes with state ("Mostrar" / "Ocultar") */
export type Handler = { run: () => void; label?: string };

/** [keys, what] rows of the cheat sheet, grouped */
export const cheatSheet = (): { group: CommandGroup; title: string; rows: [string, string][] }[] => {
  const order = Object.keys(GROUP_LABEL) as CommandGroup[];
  return order
    .map((g) => ({ group: g, title: GROUP_LABEL[g], rows: COMMANDS.filter((c) => c.group === g && c.keys).map((c) => [c.keys!, c.label] as [string, string]) }))
    .filter((g) => g.rows.length);
};

/** palette entries for the commands that have a handler right now, in registry order */
export const paletteCommands = (handlers: Partial<Record<string, Handler | undefined>>, fmtKeys: (k: string) => string = (k) => k) =>
  COMMANDS.flatMap((c) => {
    const h = handlers[c.id];
    if (!h || c.palette === false) return [];
    return [{ id: c.id, label: h.label ?? c.label, hint: c.keys ? fmtKeys(c.keys) : (c.hint ?? "ação"), run: h.run }];
  });
