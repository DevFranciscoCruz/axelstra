import { dict, count } from "./i18n";
import { enAP } from "./strings.sources.en";
import { ptBRAP } from "./strings.sources.ptbr";
import type { NameProblem, UrlProblem } from "./addProject/logic";

/** pt-PT texts of the Add project dialog: local folder, clone, GitHub, new project, remote and discovered repositories. */
export const ptAP = {
  title: "Adicionar projeto",
  sources: "De onde vem o projeto?",
  tab: {
    local: "Pasta local",
    clone: "Clonar repositório",
    github: "GitHub",
    new: "Novo projeto",
    remote: "Remoto",
    discover: "Descobertos",
  },
  hint: {
    local: "Uma pasta que já existe neste computador.",
    clone: "Um repositório git a partir de um endereço.",
    github: "Um dos teus repositórios no GitHub.",
    new: "Uma pasta nova com git iniciado.",
    remote: "Uma pasta num host WSL ou SSH.",
    discover: "Repositórios que já tens no disco.",
  },
  busy: "A trabalhar…",
  unknownMethod: "O daemon em execução é antigo demais para isto. Reinicia o Axelstra.",

  // folder field
  folder: "Pasta onde guardar",
  folderPh: "Escolhe ou escreve o caminho",
  folderBrowse: "Escolher…",
  folderPickTitle: "Escolhe a pasta onde guardar o projeto",
  folderPickFailed: "Não foi possível abrir a escolha de pastas. Escreve o caminho.",
  folderRequired: "Escolhe a pasta onde guardar.",

  // local
  localPick: "Escolher pasta…",
  localDrop: "Larga aqui uma pasta",
  localDropFallback: (name: string) =>
    `Por segurança, o Axelstra só adiciona pastas escolhidas na janela do sistema. Escolhe a pasta «${name}» ali.`,
  localDropZone: "Zona para largar uma pasta",

  // clone
  url: "Endereço do repositório",
  urlPh: "https://github.com/dono/repositorio.git",
  urlHint:
    "https://, ssh:// ou git@servidor:dono/repo. O git usa as credenciais que já tens; o Axelstra nunca pede nem guarda tokens.",
  urlProblem: {
    empty: "Escreve o endereço do repositório.",
    space: "O endereço não pode ter espaços.",
    dash: "O endereço não pode começar por «-».",
    scheme: "Usa https://, ssh:// ou git@servidor:caminho.",
  } as Record<UrlProblem, string>,
  folderName: "Nome da pasta",
  folderNameHint: "Sugerido a partir do endereço.",
  branch: "Branch (opcional)",
  branchPh: "o branch por omissão",
  depth: "Profundidade (opcional)",
  depthPh: "tudo",
  depthHint: "Um número, como 1, para um clone superficial e rápido.",
  depthBad: "A profundidade é um número inteiro a partir de 1.",
  destLine: (path: string) => `Fica em ${path}`,
  clone: "Clonar",
  cloning: "A clonar…",
  cancelClone: "Cancelar clonagem",
  progress: "Progresso da clonagem",
  starting: "A iniciar…",
  background: "A clonagem continua em segundo plano se fechares esta janela.",
  cloned: (name: string) => `Projeto ${name} clonado e adicionado.`,
  cloneCancelled: "Clonagem cancelada.",
  cloneFailed: (why: string) => `A clonagem falhou: ${why}`,

  // github
  filter: "Filtrar repositórios",
  filterPh: "Nome ou descrição",
  ghList: "Repositórios do GitHub",
  ghLoading: "A carregar os teus repositórios…",
  ghEmpty: "Nenhum repositório corresponde ao filtro.",
  ghNone: "Não há repositórios na tua conta.",
  ghNoGh: "A ferramenta gh do GitHub não está instalada. Instala-a em cli.github.com e volta a tentar.",
  ghNotAuthed: "O gh ainda não tem sessão iniciada. Corre «gh auth login» num terminal e volta a tentar.",
  ghRetry: "Tentar de novo",
  ghPrivate: "Privado",
  ghCount: (n: number) => count(n, "repositório", "repositórios"),
  ghHint: "Setas para escolher, Enter ou clique para clonar para a pasta acima.",
  ghCloneTip: (name: string) => `Clonar ${name}`,
  ghFailed: (why: string) => `Não foi possível listar os repositórios: ${why}`,

  // new project
  newName: "Nome do projeto",
  newNamePh: "o-meu-projeto",
  nameProblem: {
    empty: "Dá um nome ao projeto.",
    sep: "O nome é uma só pasta: sem «/» nem «\\».",
    dots: "O nome não pode ser «.» nem «..».",
    reserved: "Esse nome é reservado pelo Windows.",
    chars: 'O nome não pode ter < > : " | ? * nem caracteres de controlo.',
    long: "O nome é demasiado longo.",
    trail: "O nome não pode acabar em ponto ou espaço.",
  } as Record<NameProblem, string>,
  readme: "Criar README.md",
  gitignore: "Criar .gitignore",
  firstCommit: "Fazer o primeiro commit",
  create: "Criar projeto",
  creating: "A criar…",
  created: (name: string) => `Projeto ${name} criado.`,
  noIdentity:
    "A pasta foi criada, mas o git não tem nome nem email configurados, por isso não houve commit. Define user.name e user.email e faz o commit depois.",

  // discovered
  dsLoading: "A procurar repositórios…",
  dsNone: "Não encontrei repositórios novos nos sítios habituais.",
  dsList: "Repositórios descobertos",
  dsHint: "Setas para escolher, Enter ou clique para adicionar.",
  dsAdd: "Adicionar",
  dsAdding: "A adicionar…",
  dsRecent: "recente",
  dsScan: "encontrado",
  dsRefresh: "Procurar de novo",
  dsAdded: (name: string) => `Projeto ${name} adicionado.`,
  dsCount: (n: number) => count(n, "repositório", "repositórios"),

  // remote suggestions
  suggestWsl: "Distros WSL detetadas",
  suggestSsh: "Hosts do ~/.ssh/config",
  suggestUse: (spec: string) => `Usar ${spec}`,
  suggestNote: "Clica para preencher a especificação do novo host.",
};

export const AP = dict<typeof ptAP>(ptAP, enAP, ptBRAP);
