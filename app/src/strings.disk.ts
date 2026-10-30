import { count, dict } from "./i18n";
import { enD } from "./strings.disk.en";
import { ptBRD } from "./strings.disk.ptbr";

/** pt-PT texts of disk hygiene (worktree sizes, "Limpar builds", the unintegrated-work warning, Settings, notices). */
export const ptD = {
  panel: {
    head: "Disco",
    volume: (root: string, free: string, total: string) => `${root} ${free} livres de ${total}`,
    low: "pouco espaço",
    loading: "A medir…",
    empty: "Nenhum worktree com tamanho medido",
    measure: "Medir de novo",
    active: "em uso",
    sizeTip: (target: string, nm: string, cache: string, reclaim: string) =>
      `target: ${target}\nnode_modules: ${nm}\ncaches: ${cache}\nLibertável: ${reclaim}`,
    clean: "Limpar builds",
    cleanTip: "Apaga target, node_modules e caches deste worktree (regeneráveis); o worktree e o trabalho ficam",
    cleanAll: "Limpar builds dos inativos",
    cleanAllTip: "Apaga os builds de todos os worktrees sem sessão nem build a correr",
    cleaning: "A limpar…",
  },
  ask: {
    title: "Limpar builds",
    ok: "Limpar",
    one: (name: string, size: string) =>
      `Apagar os builds de ${name} (target, node_modules, caches) e libertar ${size}? O worktree e o código ficam; o próximo build reconstrói.`,
    all: (n: number, size: string) =>
      `Apagar os builds de ${count(n, "worktree inativo", "worktrees inativos")} e libertar ${size}? Os worktrees e o código ficam; o próximo build reconstrói.`,
  },
  result: {
    nothing: "Nada para libertar",
    freed: (size: string) => `Builds apagados: ${size} libertados`,
    failed: (why: string) => `Não foi possível limpar os builds: ${why}`,
  },
  stale: {
    head: "Trabalho por integrar parado",
    line: (branch: string, commits: number, into: string, ago: string, size: string) =>
      `${branch}: ${count(commits, "commit", "commits")} fora do ${into}, inativo ${ago}, ${size}`,
    integrate: "Integrar",
    open: "Abrir",
  },
  notify: {
    kind: "Disco",
    artifacts: (branch: string, size: string) => `Builds de ${branch} apagados: ${size} libertados`,
    artifactsAgg: (n: number, size: string) =>
      `Builds de ${count(n, "worktree inativo apagados", "worktrees inativos apagados")}: ${size} libertados`,
    pending: (branch: string, holder: string) =>
      holder
        ? `Worktree ${branch} não foi removido: a pasta está presa por ${holder}`
        : `Worktree ${branch} não foi removido: a pasta está presa por outro processo`,
  },
  settings: {
    title: "Disco e builds",
    note: "Os builds (target, node_modules, caches) de worktrees parados são apagados sozinhos; o worktree e o trabalho nunca. Com pouco espaço, apaga primeiro os mais antigos e maiores.",
    enabled: "Vigiar o espaço livre do disco",
    minFreeGb: "Pouco espaço abaixo de (GB)",
    minFreePct: "ou abaixo de (% do disco)",
    marginGb: "Libertar até mais (GB)",
    quietHours: "Apagar os builds de um worktree parado há (horas)",
  },
};

export const D = dict<typeof ptD>(ptD, enD, ptBRD);
