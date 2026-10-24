/** pt-PT texts of the token budgets (Settings section, status bar, popover, alerts). */
import { count } from "./strings";

const period = { daily: "diário", weekly: "semanal" } as const;

export const B = {
  title: "Orçamentos de tokens",
  note: "Avisa quando um worktree gasta demasiados tokens num dia ou numa semana: 80% é um aviso, 100% é excedido. Conta entrada, saída e escrita de cache dos agentes.",
  defaultRow: "Todos os worktrees",
  defaultHint: "Vale para os worktrees sem limite próprio.",
  daily: "Limite por dia",
  weekly: "Limite por semana",
  limitHint: "Tokens, por exemplo 5M ou 800k. Vazio = sem limite.",
  invalid: (v: string) => `«${v}» não é um número de tokens válido. Escreve por exemplo 5M, 800k ou 1500000.`,
  autoPause: "Interromper os agentes ao exceder o limite",
  autoPauseWarn:
    "Envia Esc aos agentes que estiverem a trabalhar no worktree quando o limite é excedido. Pára o turno em curso e pode deixar trabalho a meio. Desligado por omissão; só interrompe se ligares aqui.",
  perWorktree: "Limites por worktree",
  perWorktreeNote: "Um limite próprio substitui o de «Todos os worktrees» nesse worktree.",
  addWorktree: "Adicionar limite para…",
  addWorktreeLabel: "Worktree a quem dar um limite próprio",
  remove: "Remover",
  removeLabel: (name: string) => `Remover o limite próprio de ${name}`,
  noWorktrees: "Sem worktrees para escolher.",
  oldDaemon: "O daemon em execução não conhece orçamentos. Reinicia o Axelstra para os usar.",
  saved: "Orçamentos guardados.",
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
    `Orçamento ${period[p]} de ${name} a ${pct}%: ${used} de ${limit} tokens`,
  overText: (p: "daily" | "weekly", name: string, used: string, limit: string) =>
    `Orçamento ${period[p]} de ${name} excedido: ${used} de ${limit} tokens`,
  paused: (n: number) => ` · ${count(n, "agente interrompido", "agentes interrompidos")}`,
};
