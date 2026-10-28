import { dict } from "./i18n";
import { enR } from "./strings.remote.en";
import { ptBRR } from "./strings.remote.ptbr";
import { count } from "./i18n";
import type { HostState } from "./api";
import type { RemoteFailure, SpecProblem } from "./hostLogic";

/** pt-PT texts of the remote hosts (M7b): badge, Settings section and the "adicionar projeto remoto" flow. */
export const ptR = {
  state: {
    idle: "desligado",
    connecting: "a ligar…",
    ok: "ligado",
    lost: "ligação perdida",
    auth: "falta autenticação",
  } as Record<HostState, string>,
  stateOf: (name: string, s: HostState): string => `Host ${name}: ${R.state[s]}`,
  badgeTip: (name: string, spec: string, s: HostState): string => `${name} (${spec}) · ${R.state[s]}`,
  reconnect: "Voltar a ligar",
  reconnectTip: (name: string) => `Voltar a ligar ao host ${name}`,
  reconnecting: (name: string) => `A ligar ao host ${name}…`,
  reconnected: (name: string) => `Ligado a ${name}.`,
  reconnectFailed: (name: string, why: string) => `Não foi possível ligar a ${name}: ${why}`,
  remoteNoDocker: "Indisponível em projetos remotos",
  openInRemote: "Abrir em… (indisponível em remotos)",
  remoteNoOpen: "Abrir externamente não está disponível em projetos remotos",
  remoteNoComputer: "O controlo do ecrã não funciona em projetos remotos",
  offline: (name: string) => `O host ${name} não está ligado.`,

  // Settings
  section: "Hosts remotos",
  sectionNote:
    "Máquinas onde corre outro daemon (uma distro WSL ou um servidor por SSH). Cada uma tem os seus projetos, worktrees e agentes.",
  none: "Ainda não há hosts.",
  remove: "Remover",
  removeTip: (name: string) => `Remover o host ${name}`,
  removeAsk: (name: string) =>
    `Remover o host ${name}? Os projetos dele deixam de aparecer aqui; no host nada é apagado.`,
  removed: (name: string) => `Host ${name} removido.`,
  addTitle: "Adicionar host",
  name: "Nome",
  namePh: "ex.: servidor",
  spec: "Especificação",
  specPh: "ssh:utilizador@servidor ou wsl:Ubuntu",
  specHint: "wsl:<distro> ou ssh:[utilizador@]servidor[:porta]. Só chave SSH ou agent: nunca pede palavra-passe.",
  nameEmpty: "Dá um nome ao host.",
  add: "Adicionar host",
  adding: "A adicionar…",
  added: (name: string) => `Host ${name} adicionado.`,
  addFailed: (why: string) => `Não foi possível adicionar o host: ${why}`,
  builds: (proto: number | undefined, build: string | undefined) =>
    [proto != null ? `proto ${proto}` : "", build ?? ""].filter(Boolean).join(" · "),
  spec_problem: {
    empty: "Escreve a especificação.",
    space: "A especificação não pode ter espaços nem quebras de linha.",
    dash: "Nenhuma parte da especificação pode começar por «-».",
    scheme: "Começa por wsl: ou ssh:.",
    distro: "Distro inválida: usa letras, números, «.», «_» e «-».",
    host: "Servidor inválido: usa um nome, um IP ou um IPv6 entre parênteses retos.",
    user: "Utilizador inválido: usa letras, números, «.», «_» e «-».",
    port: "A porta tem de estar entre 1 e 65535.",
  } as Record<SpecProblem, string>,

  // add remote project
  addRemoteTitle: "Adicionar projeto remoto",
  addProjectTitle: "Adicionar projeto",
  chooseSource: "Onde está o projeto?",
  sourceLocal: "Pasta local",
  sourceLocalHint: "Escolhe uma pasta neste computador.",
  sourceRemote: "Projeto remoto",
  sourceRemoteHint: "Uma pasta num host WSL ou SSH.",
  back: "Voltar",
  host: "Host",
  hostNone: "Escolhe um host",
  newHost: "Adicionar novo host…",
  path: "Pasta no host",
  pathPh: "/home/utilizador/projeto",
  pathHint: "Caminho absoluto no host. Enter abre a pasta selecionada na lista.",
  pathEmpty: "Escreve o caminho da pasta.",
  pathRelative: "O caminho tem de ser absoluto (começa por «/» ou por uma unidade, como «C:»).",
  browse: "Procurar",
  browseLabel: "Pastas em",
  crumbs: "Caminho atual",
  up: "Pasta acima",
  home: "Pasta pessoal",
  folders: "Pastas",
  emptyDir: "Sem subpastas.",
  gitTip: "Repositório git",
  truncated: "Lista cortada: escreve o caminho completo.",
  browsing: "A listar…",
  browseCount: (n: number) => count(n, "pasta", "pastas"),
  use: "Usar esta pasta",
  confirm: "Adicionar projeto",
  confirming: "A adicionar…",
  projectAdded: (name: string, host: string) => `Projeto ${name} adicionado em ${host}.`,
  cancelled: "Adição cancelada.",
  waitingHost: (name: string) => `A ligar a ${name}…`,
  failure: {
    auth: "Falta autenticação. Usa uma chave SSH ou o ssh-agent: o Axelstra nunca pede palavra-passe.",
    "no-axelstra":
      "O comando axelstra não existe no host (ou não está no PATH de uma sessão não interativa). Instala-o lá.",
    "no-daemon": "O daemon não está a correr no host. Inicia-o lá com «axelstra daemon» e volta a ligar.",
    unreachable: "Não foi possível chegar ao host (rede, nome ou tempo esgotado).",
    other: "Falhou a ligação ao host.",
  } as Record<RemoteFailure, string>,
  detail: "Detalhe",
};

export const R = dict<typeof ptR>(ptR, enR, ptBRR);
