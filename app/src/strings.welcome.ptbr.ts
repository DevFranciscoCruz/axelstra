import type { Widen } from "./i18n";
import type { ptW } from "./strings.welcome";

/** Brazilian Portuguese texts of the empty main-area screen (no project, or no worktree chosen). */
export const ptBRW: Widen<typeof ptW> = {
  title: "Bem-vindo ao Axelstra",
  flow: "Adicione um projeto, crie um worktree e abra um agente.",
  add: "Adicionar projeto",
  pickTitle: "Escolha um worktree",
  pickFlow: (project?: string) =>
    project
      ? `Abra um worktree de ${project} ou crie um novo para uma tarefa.`
      : "Abra um worktree ou crie um novo para uma tarefa.",
  newWorktree: "Novo worktree",
  detached: "(desanexado)",
  mainTag: "principal",
  palette: "paleta de comandos",
  shortcuts: "atalhos",
  needs: "Para começar",
  needGit: { name: "git", why: "obrigatório, sem ele não há worktrees" },
  needAgent: { name: "claude", why: "ou outro agente (por exemplo codex)" },
  toolFound: "instalado",
  toolMissing: "não encontrado",
  needGh: { name: "gh", why: "só é necessário para PRs" },
};
