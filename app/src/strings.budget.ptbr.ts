import { type Widen, count } from "./i18n";
import type { ptB } from "./strings.budget";

/** Brazilian Portuguese texts of the token budgets (Settings section, status bar, popover, alerts). */
const period = { daily: "diário", weekly: "semanal" } as const;

export const ptBRB: Widen<typeof ptB> = {
  title: "Orçamentos de tokens",
  note: "Avisa quando um worktree gasta tokens demais em um dia ou em uma semana: 80% é um aviso, 100% é excedido. Conta entrada, saída e gravação de cache dos agentes.",
  defaultRow: "Todos os worktrees",
  defaultHint: "Vale para os worktrees sem limite próprio.",
  daily: "Limite por dia",
  weekly: "Limite por semana",
  limitHint: "Tokens, por exemplo 5M ou 800k. Vazio = sem limite.",
  invalid: (v: string) => `«${v}» não é um número de tokens válido. Digite, por exemplo, 5M, 800k ou 1500000.`,
  autoPause: "Interromper os agentes ao exceder o limite",
  autoPauseWarn:
    "Envia Esc aos agentes que estiverem trabalhando no worktree quando o limite é excedido. Interrompe o turno em andamento e pode deixar trabalho pela metade. Desativado por padrão; só interrompe se você ativar aqui.",
  perWorktree: "Limites por worktree",
  perWorktreeNote: "Um limite próprio substitui o de «Todos os worktrees» nesse worktree.",
  addWorktree: "Adicionar limite para…",
  addWorktreeLabel: "Worktree que receberá um limite próprio",
  remove: "Remover",
  removeLabel: (name: string) => `Remover o limite próprio de ${name}`,
  noWorktrees: "Sem worktrees para escolher.",
  oldDaemon: "O daemon em execução não conhece orçamentos. Reinicie o Axelstra para usá-los.",
  saved: "Orçamentos salvos.",
  // status bar and popover
  periodName: (p: "daily" | "weekly") => period[p],
  meterTip: (p: "daily" | "weekly", name: string, used: string, limit: string, pct: number) =>
    `Orçamento ${period[p]} de ${name}: ${used} de ${limit} tokens (${pct}%)`,
  meterShort: { daily: "dia", weekly: "sem." },
  levelWord: { ok: "dentro do limite", warn: "perto do limite", over: "excedido" },
  chipOver: "Há orçamentos de tokens excedidos",
  chipWarn: "Há orçamentos de tokens perto do limite",
  // alerts
  warnText: (p: "daily" | "weekly", name: string, pct: number, used: string, limit: string) =>
    `Orçamento ${period[p]} de ${name} em ${pct}%: ${used} de ${limit} tokens`,
  overText: (p: "daily" | "weekly", name: string, used: string, limit: string) =>
    `Orçamento ${period[p]} de ${name} excedido: ${used} de ${limit} tokens`,
  paused: (n: number) => ` · ${count(n, "agente interrompido", "agentes interrompidos")}`,
};
