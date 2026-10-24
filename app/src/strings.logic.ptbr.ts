import type { DeepPartial, Widen } from "./i18n";
import type { ptL } from "./strings.logic";

/**
 * Brazilian Portuguese: only what differs from pt-PT (the vocabulary of the commands, tabs and dots). Every key left out
 * falls back to pt-PT, so the language stays usable while it is filled in; `i18n.test.ts` checks that no key is invented here.
 */
export const ptBRL: DeepPartial<Widen<typeof ptL>> = {
  groups: { separadores: "Abas e terminais", worktree: "Worktrees e projetos" },
  cmd: {
    settings: { label: "Configurações" },
    shortcuts: { label: "Atalhos de teclado" },
    panelFiles: { label: "Painel: arquivos" },
    theme: { label: "Mudar o tema" },
    language: { label: "Mudar o idioma" },
    nextTab: { label: "Aba seguinte / anterior" },
    gotoTab: { label: "Ir para a aba n" },
    reopen: { label: "Reabrir a última aba fechada" },
    rename: { label: "Renomear a aba" },
    termFind: { label: "Pesquisar no terminal" },
    newTerminal: { label: "Novo terminal" },
    computer: { label: "Controle da tela pelos agentes" },
    tabRename: { label: "Renomear uma aba", keys: "Clique duplo na aba" },
    tabClose: { label: "Fechar uma aba", keys: "Clique com o botão do meio na aba" },
  },
  dot: { working: "Trabalhando", idle: "Parado", waiting: "Aguardando resposta" },
  session: (id: number) => `Sessão ${id}`,
  editor: {
    binary: "Arquivo binário: não pode ser aberto no editor.",
    tooLarge: "Arquivo grande demais para o editor.",
  },
  ago: {
    now: "agora",
    min: (n: number) => `há ${n} min`,
    hour: (n: number) => `há ${n} h`,
    day: (n: number) => `há ${n} d`,
  },
};
