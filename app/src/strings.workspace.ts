import { dict } from "./i18n";
import { enWS } from "./strings.workspace.en";
import { count, plural } from "./i18n";

/** pt-PT texts of the workspace export / import section of Settings. */
export const ptWS = {
  title: "Configuração do espaço de trabalho",
  note: "Guarda os projetos, as automatizações, as definições e os cartões kanban num ficheiro, ou traz-os de outra máquina. Nunca inclui tokens, sessões nem registos.",
  export: "Exportar…",
  import: "Importar…",
  exportPick: "Guardar a configuração",
  importPick: "Escolher o ficheiro de configuração",
  mapPick: "Escolhe a pasta deste projeto nesta máquina",
  exported: (path: string) => `Configuração guardada em ${path}`,
  noSupport: "Este daemon ainda não exporta nem importa a configuração. Atualiza e reinicia o Axelstra.",
  dialogTitle: "Importar configuração",
  fileLine: (file: string, at: string) => (at ? `${file} · exportado em ${at}` : file),
  kinds: { projects: "Projetos", automations: "Automatizações", settings: "Definições", cards: "Cartões" },
  nothing: "nada",
  nNew: (n: number) => count(n, "novo", "novos"),
  nPresent: (n: number) => `${n} ${plural(n, "já existe igual", "já existem iguais")}`,
  nConflict: (n: number) => count(n, "diferente", "diferentes"),
  nSkipped: (n: number) => count(n, "ignorado", "ignorados"),
  localOnly: (n: number) => count(n, "só existe aqui", "só existem aqui"),
  reasons: {
    missing: "a pasta não existe nesta máquina",
    forbidden: "a pasta não pode ser um projeto",
    duplicate: "outra entrada do ficheiro usa a mesma pasta",
    "project skipped": "o projeto foi ignorado",
    unsafe: "a pasta .kanban é uma ligação simbólica",
  } as Record<string, string>,
  reasonOther: "não pode ser importado",
  moreSkipped: (n: number) => `e mais ${count(n, "ignorado", "ignorados")}`,
  mapButton: "Escolher pasta…",
  mapped: "Pastas mapeadas",
  mapClear: "Limpar mapeamentos",
  risky: (n: number) =>
    `${count(n, "cartão corre", "cartões correm")} com as permissões desligadas ou com um comando de aceitação. Lê ${plural(n, "o cartão", "os cartões")} antes de o${plural(n, "", "s")} executares.`,
  mode: "Modo",
  modeMerge: "Juntar",
  modeMergeHint: "Acrescenta o que falta e mantém o que já existe.",
  modeReplace: "Substituir",
  modeReplaceHint: "Troca os projetos, as automatizações e as definições pelos do ficheiro.",
  conflicts: "Quando é diferente",
  conflictsKeep: "Manter o meu",
  conflictsOverwrite: "Usar o do ficheiro",
  replaceWarn: (n: number) =>
    `Substituir tira da lista ${count(n, "projeto que não está", "projetos que não estão")} no ficheiro. Os ficheiros de estado anteriores ficam guardados como .1. Os cartões só são substituídos se escolheres «Usar o do ficheiro».`,
  replaceWord: "substituir",
  replaceType: (w: string) => `Escreve «${w}» para confirmar`,
  cancel: "Cancelar",
  confirm: "Importar",
  importing: "A importar…",
  done: (o: {
    projects_added: number;
    automations_added: number;
    automations_updated: number;
    settings_set: number;
    cards_written: number;
    cards_kept: number;
  }) =>
    [
      count(o.projects_added, "projeto novo", "projetos novos"),
      count(o.automations_added + o.automations_updated, "automatização", "automatizações"),
      count(o.settings_set, "definição", "definições"),
      `${count(o.cards_written, "cartão escrito", "cartões escritos")}${o.cards_kept ? `, ${count(o.cards_kept, "mantido", "mantidos")}` : ""}`,
    ].join(" · "),
  doneTitle: "Configuração importada: ",
  restartHint: " Reinicia o Axelstra para aplicar as definições.",
};

export const WS = dict(ptWS, enWS);
