import { count, dict, plural } from "./i18n";
import { enU } from "./strings.ui.en";
import { ptBRU } from "./strings.ui.ptbr";

/** pt-PT texts that sit directly in small components: close-window dialog, dispatcher, shortcuts, right tabs, loading, error boundary, worktree dialogs, reports. */
export const ptU = {
  boundary: {
    whole: "O Axelstra encontrou um erro.",
    part: (name: string) => `Esta parte (${name}) encontrou um erro.`,
    terminalsKeep: "Os terminais continuam a correr.",
    copyDiag: "Copiar diagnóstico",
    reload: "Recarregar",
  },
  loading: {
    slow: "Está a demorar mais do que o normal.",
  },
  closeConfirm: {
    label: "Fechar janela",
    title: "Fechar o Axelstra?",
    none: "Não há sessões em execução.",
    keepOn: (n: number) =>
      `${count(n, "sessão continua", "sessões continuam")} a correr em segundo plano (e consomem recursos) até voltar a abrir a aplicação.`,
    keepOff: (n: number) =>
      `${count(n, "sessão é fechada", "sessões são fechadas")} e nada fica a consumir. ${plural(n, "Volta", "Voltam")} quando abrir a aplicação outra vez; os agentes retomam a conversa.`,
    keepBackground: "Manter sessões e automatizações a correr em segundo plano",
    dontAsk: "Não perguntar novamente",
  },
  dispatcher: {
    title: "Despachante",
    note: "Cada pedido feito na main vai para o worktree do seu tópico. Aqui vês para onde foi cada um.",
    empty: "Ainda não há pedidos despachados. Pede algo ao agente da main.",
    waiting: "à espera de resposta",
    working: "a trabalhar",
    idle: "parado",
    noAgent: "sem agente",
    archived: "arquivado",
    openWorktree: "Abrir este worktree",
  },
  shortcuts: {
    title: "Atalhos de teclado",
  },
  rightTabs: {
    label: "Painel direito",
    files: "Ficheiros",
    changes: "Alterações",
    prs: "PRs",
    hide: "Esconder painel direito (Ctrl+Shift+B)",
  },
  reports: {
    list: "Relatórios dos agentes deste worktree",
    open: "Abrir o relatório",
    status: { ok: "concluído", nota: "com notas", aberto: "sem relatório" },
  },
  createWorktree: {
    title: "Novo worktree",
    project: "Projeto",
    name: "Nome",
    namePh: "nome do worktree (também o nome do branch)",
    agent: "Agente",
    noAgent: "Nenhum (só o worktree)",
    create: "Criar worktree",
  },
  renameWorktree: {
    title: "Mudar o nome do worktree",
    name: "Nome",
    submit: "Mudar o nome",
  },
} as const;

export const U = dict<typeof ptU>(ptU, enU, ptBRU);
