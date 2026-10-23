import type { CheckItem, ShipStatus } from "./api";

/** how often the PR state is read while the panel is on screen and the window has focus */
export const SHIP_POLL_MS = 20_000;

/** [icon, word]: the word is the cue that does not depend on colour */
export const CI_LABEL: Record<ShipStatus["ci"], [string, string]> = {
  pass: ["check", "CI a passar"],
  fail: ["x", "CI a falhar"],
  pending: ["ellipsis", "CI a correr"],
  none: ["", "Sem CI"],
};
export const CHECK_LABEL: Record<CheckItem["state"], [string, string]> = {
  pass: ["check", "passou"],
  fail: ["x", "falhou"],
  pending: ["ellipsis", "a correr"],
  skipped: ["", "ignorado"],
};
export const REVIEW_LABEL: Record<ShipStatus["review"], string> = {
  approved: "Aprovado",
  changes: "Alterações pedidas",
  review: "Falta revisão",
  "": "Sem revisão pedida",
};

/** one phrase for "can this be merged": what blocks it, or that nothing does */
export const mergeLabel = (s: ShipStatus): string => {
  if (s.pr_state === "merged") return "Merge feito";
  if (s.pr_state === "closed") return "Fechado sem merge";
  if (s.draft) return "Rascunho";
  if (s.mergeable === "no" || s.merge_state === "dirty") return "Com conflitos";
  if (s.can_merge) return "Pronto para merge";
  if (s.merge_state === "behind") return "Branch desatualizado";
  if (s.ci === "fail") return "Bloqueado pelos checks";
  if (s.ci === "pending") return "À espera dos checks";
  if (s.merge_state === "blocked") return "Bloqueado (revisão ou regras do repositório)";
  return "A calcular…";
};

/** auto-merge makes sense while the PR is open and not yet mergeable: GitHub merges it when the checks pass */
export const canAutoMerge = (s: ShipStatus): boolean => s.pr_state === "open" && !s.draft && !s.can_merge && s.mergeable !== "no";

export const problemText = (s: Pick<ShipStatus, "problem" | "detail">): { text: string; fix: string } => {
  switch (s.problem) {
    case "missing":
      return { text: "O GitHub CLI (gh) não está instalado.", fix: "Instala o gh (https://cli.github.com) e corre `gh auth login` num terminal." };
    case "auth":
      return { text: "O gh não tem sessão iniciada no GitHub.", fix: "Corre `gh auth login` num terminal e depois atualiza." };
    case "nopr":
      return { text: "Ainda não há pull request para este branch.", fix: "Faz push e cria o PR com “Criar PR” acima." };
    default:
      return { text: s.detail || "Não foi possível ler o estado do pull request.", fix: "Atualiza daqui a pouco; se persistir, confirma que `gh pr view` funciona neste worktree." };
  }
};

/** the window must be showing the panel and have focus, otherwise a poll is wasted */
export const shouldPoll = (hidden: boolean, focused: boolean) => !hidden && focused;

export const mergeQuestion = (s: Pick<ShipStatus, "number" | "title">, branch: string, deleteBranch: boolean) =>
  `Fazer merge (squash) do pull request #${s.number}?\n\n${s.title}\n\nO branch "${branch}" entra no branch principal.${deleteBranch ? " O branch remoto será apagado." : ""} Isto não se desfaz.`;

export const autoMergeQuestion = (s: Pick<ShipStatus, "number" | "title">) =>
  `Ativar o auto-merge (squash) do pull request #${s.number}?\n\n${s.title}\n\nO GitHub faz o merge sozinho quando os checks passarem e as revisões estiverem cumpridas.`;
