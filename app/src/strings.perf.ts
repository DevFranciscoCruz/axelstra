import { count, dict, fmtFixed, fmtNumber } from "./i18n";
import { enPF } from "./strings.perf.en";
import { ptBRPF } from "./strings.perf.ptbr";

/** pt-PT texts of the Performance section of Settings (antivirus and indexer exclusions of the .axelstra-wt folders). */
export const ptPF = {
  title: "Desempenho",
  intro:
    "Os worktrees criam e apagam muitos ficheiros pequenos, e o antivírus e o indexador do sistema leem cada um. Excluir as pastas .axelstra-wt desses leitores acelera git, instalações e compilações.",
  autoIndexer: "Sem fazer nada: o Axelstra já marca estas pastas para não serem indexadas pelo sistema.",
  tradeoff:
    "Compromisso: uma pasta excluída do Defender deixa de ser verificada em tempo real. Só recomendamos isto para as pastas .axelstra-wt do Axelstra, nunca para os teus projetos.",
  loading: "A ler o estado…",
  loadingSlow: "A leitura está a demorar. Atualiza daqui a pouco.",
  unavailable: "Não foi possível ler o estado das exclusões.",
  macos:
    "No macOS o Axelstra coloca um ficheiro .metadata_never_index em .axelstra-wt, para o Spotlight o ignorar. Não há mais nada a configurar.",
  linux:
    "No Linux o Axelstra não altera nada. Se o tracker ou o baloo indexar os worktrees, exclui a pasta .axelstra-wt nas definições do indexador.",
  noFolders: "Ainda não há pastas .axelstra-wt: aparecem quando crias o primeiro worktree.",
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
    inactive: "O Defender não é o antivírus ativo: outro produto (ou nenhum) verifica os ficheiros.",
    unavailable: "Não foi possível consultar o Defender.",
    na: "Sem Defender neste sistema.",
  },
  realtimeOn: "Verificação em tempo real ligada.",
  realtimeOff: "Verificação em tempo real desligada.",
  tamperOn: "A Proteção contra Adulteração está ligada: o Defender pode recusar a exclusão.",
  unreadable:
    "Sem direitos de administrador o Axelstra não consegue ler as exclusões do Defender; só sabe as que ele próprio aplicou.",
  consent: "Percebo o compromisso e quero excluir as pastas .axelstra-wt do Defender",
  uac: "O Windows vai pedir permissão de administrador (UAC). Nada é alterado antes de aceitares.",
  apply: "Aplicar…",
  revert: "Reverter",
  revertNote: "Reverter remove só as exclusões que o Axelstra aplicou, nunca as tuas.",
  running: "A aguardar o pedido de administrador…",
  applied: (n: number, same: number) =>
    `Exclusão aplicada a ${count(n, "pasta", "pastas")}` +
    (same ? ` (${count(same, "já estava excluída", "já estavam excluídas")})` : "."),
  appliedNone: "Nada a alterar: as pastas já estavam excluídas.",
  reverted: (n: number) => `Exclusão removida de ${count(n, "pasta", "pastas")}.`,
  revertedNone: "Nada a reverter.",
  failed: (detail: string) => `Não foi possível concluir: ${detail}`,
  diagTitle: "Uso do antivírus e do indexador",
  diagNote: "Leitura única do processador e da memória dos processos do Defender e do indexador do Windows.",
  diagRefresh: "Atualizar",
  diagNone: "Nenhum destes processos está a correr.",
  diagNa: "Disponível apenas no Windows.",
  diagProc: (name: string, cpu: number | null, rss: number | null) =>
    `${name}: ${cpu === null ? "—" : `${fmtFixed(cpu, 0)} % de um núcleo`} · ${rss === null ? "—" : `${fmtNumber(Math.round(rss / 1048576))} MB`}`,
  hintTitle: "Worktrees mais rápidos?",
  hintSub:
    "O Defender está a verificar a pasta dos worktrees em tempo real. Vê as opções em Definições > Desempenho. Nada é alterado sem o teu clique.",
  hintOpen: "Ver opções",
  hintDismiss: "Não mostrar mais",
  hintLabel: "Sugestão de desempenho",
};

export const PF = dict<typeof ptPF>(ptPF, enPF, ptBRPF);
