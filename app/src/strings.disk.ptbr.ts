import { type Widen, count } from "./i18n";
import type { ptD } from "./strings.disk";

/** Brazilian Portuguese texts of disk hygiene (worktree sizes, "Limpar builds", the unintegrated-work warning, Settings, notices). */
export const ptBRD: Widen<typeof ptD> = {
  panel: {
    head: "Disco",
    volume: (root: string, free: string, total: string) => `${root} ${free} livres de ${total}`,
    low: "pouco espaço",
    loading: "Medindo…",
    empty: "Nenhum worktree com tamanho medido",
    measure: "Medir de novo",
    active: "em uso",
    sizeTip: (target: string, nm: string, cache: string, reclaim: string) =>
      `target: ${target}\nnode_modules: ${nm}\ncaches: ${cache}\nLiberável: ${reclaim}`,
    clean: "Limpar builds",
    cleanTip:
      "Exclui target, node_modules e caches deste worktree (podem ser gerados de novo); o worktree e o trabalho continuam",
    cleanAll: "Limpar builds dos inativos",
    cleanAllTip: "Exclui os builds de todos os worktrees sem sessão nem build rodando",
    cleaning: "Limpando…",
  },
  ask: {
    title: "Limpar builds",
    ok: "Limpar",
    one: (name: string, size: string) =>
      `Excluir os builds de ${name} (target, node_modules, caches) e liberar ${size}? O worktree e o código continuam; o próximo build gera tudo de novo.`,
    all: (n: number, size: string) =>
      `Excluir os builds de ${count(n, "worktree inativo", "worktrees inativos")} e liberar ${size}? Os worktrees e o código continuam; o próximo build gera tudo de novo.`,
  },
  result: {
    nothing: "Nada para liberar",
    freed: (size: string) => `Builds excluídos: ${size} liberados`,
    failed: (why: string) => `Não foi possível limpar os builds: ${why}`,
  },
  stale: {
    head: "Trabalho não integrado parado",
    line: (branch: string, commits: number, into: string, ago: string, size: string) =>
      `${branch}: ${count(commits, "commit", "commits")} fora da ${into}, inativo ${ago}, ${size}`,
    integrate: "Integrar",
    open: "Abrir",
  },
  notify: {
    kind: "Disco",
    artifacts: (branch: string, size: string) => `Builds de ${branch} excluídos: ${size} liberados`,
    artifactsAgg: (n: number, size: string) =>
      `Builds de ${count(n, "worktree inativo excluídos", "worktrees inativos excluídos")}: ${size} liberados`,
    pending: (branch: string, holder: string) =>
      holder
        ? `O worktree ${branch} não foi removido: a pasta está presa por ${holder}`
        : `O worktree ${branch} não foi removido: a pasta está presa por outro processo`,
  },
  settings: {
    title: "Disco e builds",
    note: "Os builds (target, node_modules, caches) de worktrees parados são excluídos automaticamente; o worktree e o trabalho nunca. Com pouco espaço, os mais antigos e maiores saem primeiro.",
    enabled: "Monitorar o espaço livre do disco",
    minFreeGb: "Pouco espaço abaixo de (GB)",
    minFreePct: "ou abaixo de (% do disco)",
    marginGb: "Liberar até mais (GB)",
    quietHours: "Excluir os builds de um worktree parado há (horas)",
  },
};
