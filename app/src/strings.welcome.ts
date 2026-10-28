import { dict } from "./i18n";
import { enW } from "./strings.welcome.en";
import { ptBRW } from "./strings.welcome.ptbr";
/** pt-PT texts of the empty main-area screen (no project, or no worktree chosen). */
export const ptW = {
  title: "Bem-vindo ao Axelstra",
  flow: "Adiciona um projeto, cria um worktree e abre um agente.",
  stepsTitle: "Como funciona",
  steps: [
    "Adiciona uma pasta com um repositório git.",
    "Cria um worktree: cada tarefa tem o seu branch e a sua pasta.",
    "Abre um agente (Claude, Codex…) nesse worktree e trabalha em paralelo.",
  ],
  offlineTitle: "Sem ligação ao daemon",
  offlineFlow: "Os teus projetos estão guardados; não aparecem porque o daemon não responde. Tenta ligar de novo ou reinicia-o.",
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
