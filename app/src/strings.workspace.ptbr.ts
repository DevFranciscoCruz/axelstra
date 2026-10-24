import { type Widen, count, plural } from "./i18n";
import type { ptWS } from "./strings.workspace";

/** Brazilian Portuguese texts of the workspace export / import section of Settings. */
export const ptBRWS: Widen<typeof ptWS> = {
  title: "Configuração do espaço de trabalho",
  note: "Salva os projetos, as automações, as configurações e os cartões kanban em um arquivo, ou traz tudo de outra máquina. Nunca inclui tokens, sessões nem logs.",
  export: "Exportar…",
  import: "Importar…",
  exportPick: "Salvar a configuração",
  importPick: "Escolher o arquivo de configuração",
  mapPick: "Escolha a pasta deste projeto nesta máquina",
  exported: (path: string) => `Configuração salva em ${path}`,
  noSupport: "Este daemon ainda não exporta nem importa a configuração. Atualize e reinicie o Axelstra.",
  dialogTitle: "Importar configuração",
  fileLine: (file: string, at: string) => (at ? `${file} · exportado em ${at}` : file),
  kinds: { projects: "Projetos", automations: "Automações", settings: "Configurações", cards: "Cartões" },
  nothing: "nada",
  nNew: (n: number) => count(n, "novo", "novos"),
  nPresent: (n: number) => `${n} ${plural(n, "já existe igual", "já existem iguais")}`,
  nConflict: (n: number) => count(n, "diferente", "diferentes"),
  nSkipped: (n: number) => count(n, "ignorado", "ignorados"),
  localOnly: (n: number) => count(n, "só existe aqui", "só existem aqui"),
  reasons: {
    missing: "a pasta não existe nesta máquina",
    forbidden: "a pasta não pode ser um projeto",
    duplicate: "outra entrada do arquivo usa a mesma pasta",
    "project skipped": "o projeto foi ignorado",
    unsafe: "a pasta .kanban é um link simbólico",
  },
  reasonOther: "não pode ser importado",
  moreSkipped: (n: number) => `e mais ${count(n, "ignorado", "ignorados")}`,
  mapButton: "Escolher pasta…",
  mapped: "Pastas mapeadas",
  mapClear: "Limpar mapeamentos",
  risky: (n: number) =>
    `${count(n, "cartão roda", "cartões rodam")} com as permissões desativadas ou com um comando de aceitação. Leia ${plural(n, "o cartão", "os cartões")} antes de executá-${plural(n, "lo", "los")}.`,
  mode: "Modo",
  modeMerge: "Mesclar",
  modeMergeHint: "Acrescenta o que falta e mantém o que já existe.",
  modeReplace: "Substituir",
  modeReplaceHint: "Troca os projetos, as automações e as configurações pelos do arquivo.",
  conflicts: "Quando for diferente",
  conflictsKeep: "Manter o meu",
  conflictsOverwrite: "Usar o do arquivo",
  replaceWarn: (n: number) =>
    `Substituir remove da lista ${count(n, "projeto que não está", "projetos que não estão")} no arquivo. Os arquivos de estado anteriores ficam salvos como .1. Os cartões só são substituídos se você escolher «Usar o do arquivo».`,
  replaceWord: "substituir",
  replaceType: (w: string) => `Digite «${w}» para confirmar`,
  cancel: "Cancelar",
  confirm: "Importar",
  importing: "Importando…",
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
      count(o.automations_added + o.automations_updated, "automação", "automações"),
      count(o.settings_set, "configuração", "configurações"),
      `${count(o.cards_written, "cartão gravado", "cartões gravados")}${o.cards_kept ? `, ${count(o.cards_kept, "mantido", "mantidos")}` : ""}`,
    ].join(" · "),
  doneTitle: "Configuração importada: ",
  restartHint: " Reinicie o Axelstra para aplicar as configurações.",
};
