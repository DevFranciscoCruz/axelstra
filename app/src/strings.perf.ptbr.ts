import { type Widen, count, fmtFixed, fmtNumber } from "./i18n";
import type { ptPF } from "./strings.perf";

/** Brazilian Portuguese texts of the Performance section of Settings. */
export const ptBRPF: Widen<typeof ptPF> = {
  title: "Desempenho",
  intro:
    "Os worktrees criam e apagam muitos arquivos pequenos, e o antivírus e o indexador do sistema leem cada um. Excluir as pastas .axelstra-wt desses leitores acelera git, instalações e compilações.",
  autoIndexer: "Sem fazer nada: o Axelstra já marca estas pastas para o sistema não indexá-las.",
  tradeoff:
    "Compromisso: uma pasta excluída do Defender deixa de ser verificada em tempo real. Só recomendamos isso para as pastas .axelstra-wt do Axelstra, nunca para os seus projetos.",
  loading: "Lendo o estado…",
  loadingSlow: "A leitura está demorando. Atualize daqui a pouco.",
  unavailable: "Não foi possível ler o estado das exclusões.",
  macos:
    "No macOS o Axelstra coloca um arquivo .metadata_never_index em .axelstra-wt, para o Spotlight ignorá-la. Não há mais nada a configurar.",
  linux:
    "No Linux o Axelstra não altera nada. Se o tracker ou o baloo indexar os worktrees, exclua a pasta .axelstra-wt nas configurações do indexador.",
  noFolders: "Ainda não há pastas .axelstra-wt: elas aparecem quando você cria o primeiro worktree.",
  foldersLabel: "Pastas de worktrees",
  folderMissing: "a pasta ainda não existe",
  indexable: {
    yes: "indexável pelo sistema",
    no: "fora do índice do sistema",
    unknown: "índice do sistema desconhecido",
  },
  excluded: {
    yes: "excluída do Defender",
    recorded: "excluída do Defender (aplicado pelo Axelstra)",
    no: "não excluída do Defender",
    unknown: "exclusão do Defender desconhecida",
    na: "sem Defender",
  },
  devDrive: "Dev Drive (ReFS): já é verificado de forma leve, não precisa de exclusão.",
  fsLine: (fs: string, drive: string) => `${drive}: ${fs}`,
  sharedBy: (names: string) => `Projetos: ${names}`,
  defenderState: {
    active: "O Defender é o antivírus ativo.",
    inactive: "O Defender não é o antivírus ativo: outro produto (ou nenhum) verifica os arquivos.",
    unavailable: "Não foi possível consultar o Defender.",
    na: "Sem Defender neste sistema.",
  },
  realtimeOn: "Verificação em tempo real ligada.",
  realtimeOff: "Verificação em tempo real desligada.",
  tamperOn: "A Proteção contra Violação está ligada: o Defender pode recusar a exclusão.",
  unreadable:
    "Sem direitos de administrador o Axelstra não consegue ler as exclusões do Defender; só sabe as que ele mesmo aplicou.",
  consent: "Entendo o compromisso e quero excluir as pastas .axelstra-wt do Defender",
  uac: "O Windows vai pedir permissão de administrador (UAC). Nada é alterado antes de você aceitar.",
  apply: "Aplicar…",
  revert: "Reverter",
  revertNote: "Reverter remove só as exclusões que o Axelstra aplicou, nunca as suas.",
  running: "Aguardando o pedido de administrador…",
  applied: (n: number, same: number) =>
    `Exclusão aplicada a ${count(n, "pasta", "pastas")}` +
    (same ? ` (${count(same, "já estava excluída", "já estavam excluídas")})` : "."),
  appliedNone: "Nada a alterar: as pastas já estavam excluídas.",
  reverted: (n: number) => `Exclusão removida de ${count(n, "pasta", "pastas")}.`,
  revertedNone: "Nada a reverter.",
  failed: (detail: string) => `Não foi possível concluir: ${detail}`,
  diagTitle: "Uso do antivírus e do indexador",
  diagNote: "Uma leitura única do processador e da memória dos processos do Defender e do indexador do Windows.",
  diagRefresh: "Atualizar",
  diagNone: "Nenhum destes processos está em execução.",
  diagNa: "Disponível apenas no Windows.",
  diagProc: (name: string, cpu: number | null, rss: number | null) =>
    `${name}: ${cpu === null ? "—" : `${fmtFixed(cpu, 0)} % de um núcleo`} · ${rss === null ? "—" : `${fmtNumber(Math.round(rss / 1048576))} MB`}`,
  hintTitle: "Worktrees mais rápidos?",
  hintSub:
    "O Defender está verificando a pasta dos worktrees em tempo real. Veja as opções em Configurações > Desempenho. Nada é alterado sem o seu clique.",
  hintOpen: "Ver opções",
  hintDismiss: "Não mostrar mais",
  hintLabel: "Sugestão de desempenho",
};
