import type { Widen } from "../i18n";

/** Texts of the message codes raised by the automatic integration of finished agent work (prefix `integrate.`; see crates/daemon/src/integrate.rs). */
export const pt = {
  "integrate.conflict":
    "Integrar {branch} em {into} dá conflitos; nada foi alterado. Abre o main do worktree e pede-lhe que junte {into} à branch e resolva os conflitos.",
  "integrate.dirty":
    "O worktree de {branch} tem alterações por guardar. Guarda-as (commit) ou descarta-as e tenta outra vez.",
  "integrate.untrusted":
    "{branch} traz código de um pull request ou não é uma branch do Axelstra: nunca é integrada sozinha. Revê-a e junta-a à mão.",
  "integrate.default_dirty":
    "O checkout principal do projeto tem alterações por guardar (ou uma junção a meio), por isso {into} não pode ser atualizada. Limpa-o e tenta outra vez.",
  "integrate.default_elsewhere":
    "{into} não está aberta em nenhum worktree, por isso não pode ser atualizada. Abre-a no checkout principal e tenta outra vez.",
  "integrate.not_ok":
    "O agente de {branch} reportou falha ou bloqueio, por isso o trabalho não foi integrado. Abre o main do worktree para ver o motivo.",
  "integrate.busy": "Ainda há um agente a trabalhar no worktree de {branch}. Espera que acabe e tenta outra vez.",
  "integrate.failed": "O git falhou ao integrar {branch}: {detail}",
  "integrate.push_failed":
    "{into} foi atualizada, mas o push falhou: {detail}. Não há mais nada a desfazer; faz o push à mão quando puderes.",
  "integrate.not_agent_branch": "{branch} não é uma branch ax/*: só essas são integradas automaticamente.",
} as const satisfies Record<string, string>;

export const en: Widen<typeof pt> = {
  "integrate.conflict":
    "Merging {branch} into {into} has conflicts; nothing was changed. Open the worktree's main and ask it to merge {into} into the branch and resolve the conflicts.",
  "integrate.dirty": "The worktree of {branch} has uncommitted changes. Commit or discard them and try again.",
  "integrate.untrusted":
    "{branch} carries code from a pull request or is not an Axelstra branch: it is never integrated by itself. Review it and merge it by hand.",
  "integrate.default_dirty":
    "The project's main checkout has uncommitted changes (or a merge in progress), so {into} cannot be updated. Clean it up and try again.",
  "integrate.default_elsewhere":
    "{into} is not checked out in any worktree, so it cannot be updated. Check it out in the main checkout and try again.",
  "integrate.not_ok":
    "The agent of {branch} reported a failure or a block, so its work was not integrated. Open the worktree's main to see why.",
  "integrate.busy": "An agent is still working in the worktree of {branch}. Wait for it to finish and try again.",
  "integrate.failed": "Git failed while integrating {branch}: {detail}",
  "integrate.push_failed":
    "{into} was updated, but the push failed: {detail}. There is nothing else to undo; push it by hand when you can.",
  "integrate.not_agent_branch": "{branch} is not an ax/* branch: only those are integrated automatically.",
};

/** Brazilian wording: written out in full, not a fallback to pt-PT. */
export const ptBR: Widen<typeof pt> = {
  "integrate.conflict":
    "Integrar {branch} em {into} gera conflitos; nada foi alterado. Abra o main do worktree e peça para ele mesclar {into} na branch e resolver os conflitos.",
  "integrate.dirty":
    "O worktree de {branch} tem alterações não commitadas. Faça o commit ou descarte-as e tente novamente.",
  "integrate.untrusted":
    "{branch} traz código de um pull request ou não é uma branch do Axelstra: ela nunca é integrada sozinha. Revise e mescle manualmente.",
  "integrate.default_dirty":
    "O checkout principal do projeto tem alterações não commitadas (ou uma mesclagem em andamento), então {into} não pode ser atualizada. Limpe-o e tente novamente.",
  "integrate.default_elsewhere":
    "{into} não está em checkout em nenhum worktree, então não pode ser atualizada. Faça o checkout dela no checkout principal e tente novamente.",
  "integrate.not_ok":
    "O agente de {branch} informou falha ou bloqueio, então o trabalho não foi integrado. Abra o main do worktree para ver o motivo.",
  "integrate.busy": "Ainda há um agente trabalhando no worktree de {branch}. Espere ele terminar e tente novamente.",
  "integrate.failed": "O git falhou ao integrar {branch}: {detail}",
  "integrate.push_failed":
    "{into} foi atualizada, mas o push falhou: {detail}. Não há mais nada a desfazer; faça o push manualmente quando puder.",
  "integrate.not_agent_branch": "{branch} não é uma branch ax/*: só essas são integradas automaticamente.",
};
