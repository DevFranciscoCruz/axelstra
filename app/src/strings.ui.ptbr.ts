import { type Widen, count, plural } from "./i18n";
import type { ptU } from "./strings.ui";

/** Brazilian Portuguese texts that sit directly in small components. */
export const ptBRU: Widen<typeof ptU> = {
  boundary: {
    whole: "O Axelstra encontrou um erro.",
    part: (name: string) => `Esta parte (${name}) encontrou um erro.`,
    terminalsKeep: "Os terminais continuam rodando.",
    copyDiag: "Copiar diagnóstico",
    reload: "Recarregar",
  },
  loading: {
    slow: "Está demorando mais do que o normal.",
  },
  closeConfirm: {
    label: "Fechar janela",
    title: "Fechar o Axelstra?",
    none: "Não há sessões em execução.",
    keepOn: (n: number) =>
      `${count(n, "sessão continua", "sessões continuam")} rodando em segundo plano (e consomem recursos) até você abrir o aplicativo novamente.`,
    keepOff: (n: number) =>
      `${count(n, "sessão é fechada", "sessões são fechadas")} e nada fica consumindo recursos. ${plural(n, "Volta", "Voltam")} quando você abrir o aplicativo outra vez; os agentes retomam a conversa.`,
    keepBackground: "Manter sessões e automações rodando em segundo plano",
    dontAsk: "Não perguntar novamente",
  },
  dispatcher: {
    title: "Despachante",
    note: "Cada pedido feito na main vai para o worktree do seu tópico. Aqui você vê para onde foi cada um.",
    empty: "Ainda não há pedidos despachados. Peça algo ao agente da main.",
    waiting: "aguardando resposta",
    working: "trabalhando",
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
    files: "Arquivos",
    changes: "Alterações",
    prs: "PRs",
    hide: "Ocultar painel direito (Ctrl+Shift+B)",
  },
  reports: {
    list: "Relatórios dos agentes deste worktree",
    open: "Abrir o relatório",
    status: { ok: "concluído", nota: "com observações", aberto: "sem relatório" },
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
    title: "Renomear o worktree",
    name: "Nome",
    submit: "Renomear",
  },
};
