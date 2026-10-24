import { dict } from "./i18n";
import { enW } from "./strings.welcome.en";
import { ptBRW } from "./strings.welcome.ptbr";
/** pt-PT texts of the empty main-area screen (no project, or no worktree chosen). */
export const ptW = {
  title: "Bem-vindo ao Axelstra",
  flow: "Adiciona um projeto, cria um worktree e abre um agente.",
  add: "Adicionar projeto",
  pickTitle: "Escolhe um worktree",
  pickFlow: (project?: string) =>
    project
      ? `Abre um worktree de ${project} ou cria um novo para uma tarefa.`
      : "Abre um worktree ou cria um novo para uma tarefa.",
  newWorktree: "Novo worktree",
  detached: "(destacado)",
  mainTag: "principal",
  palette: "paleta de comandos",
  shortcuts: "atalhos",
  needs: "Para começar",
  needGit: { name: "git", why: "obrigatório, sem ele não há worktrees" },
  needAgent: { name: "claude", why: "ou outro agente (por exemplo codex)" },
  toolFound: "instalado",
  toolMissing: "não encontrado",
  needGh: { name: "gh", why: "só é preciso para PRs" },
} as const;

export const W = dict<typeof ptW>(ptW, enW, ptBRW);
