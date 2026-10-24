/** UI texts of the settings screen and the terminal, in one place and in pt-PT (gestor, ligar, ficheiro, ecrã). */
export const S = {
  settings: "Definições",
  close: "Fechar",
  loading: "A carregar…",
  retry: "Tentar de novo",
  restartDaemon: "Reiniciar daemon",
  restartingDaemon: "A reiniciar o daemon…",
  restartConfirm: "Reiniciar o daemon fecha os terminais abertos; os agentes são retomados depois do reinício. Continuar?",
  restartTitle: "Reiniciar o daemon",
  restartFailed: "Não foi possível reiniciar o daemon: ",
  oldDaemon: "O daemon em execução é de uma versão anterior e não conhece este pedido. Reinicia-o para carregar as definições.",
  loadFailed: "Não foi possível carregar as definições: ",

  replies: "Respostas dos agentes",
  repliesNote: "Poupa tokens e encurta os relatórios dos workers. Vale para agentes novos.",
  terse: {
    off: { label: "Normais", hint: "Os agentes respondem como sempre." },
    short: { label: "Curtas", hint: "Resposta primeiro, sem rodeios nem resumo no fim." },
    tiny: { label: "Mínimas", hint: "Frases soltas, no máximo três frases, só o essencial." },
  },

  pm: "Gestor de pacotes",
  pmNote: "Só aparecem os que estão instalados nesta máquina. O lockfile do projeto tem prioridade sobre esta escolha.",
  pmNone: "Nenhum encontrado no PATH (bun, pnpm, yarn, npm).",
  auto: "Automático",
  autoInstall: "Instalar dependências ao criar um worktree (projetos com package.json e sem comando de setup)",

  notify: "Notificações",
  notifyNote: "Avisos do sistema quando um agente termina ou precisa de ti.",
  notifySound: "Tocar um som com a notificação",
  notifyMode: {
    all: { label: "Todas", hint: "Avisa quando um agente termina ou fica à espera." },
    waiting: { label: "Só à espera", hint: "Avisa apenas quando um agente fica à espera de resposta." },
    off: { label: "Nenhuma", hint: "Sem notificações do sistema." },
  },

  shell: "Shell dos terminais",
  shellNote: "Vale para terminais e agentes novos. «Automático» usa o shell predefinido do sistema.",
  shells: { auto: "Automático", pwsh: "PowerShell 7", powershell: "Windows PowerShell", bash: "Bash", wsl: "WSL", cmd: "CMD" },

  appearance: "Aparência",
  appearanceNote: "Tema da aplicação. Os terminais e o editor seguem-no.",
  appTheme: "Tema da aplicação",
  appThemes: { auto: "Sistema", dark: "Escuro", light: "Claro" },

  terminal: "Terminal",
  terminalNote: "Aparência dos terminais. Ctrl+= / Ctrl+- / Ctrl+0 ajustam o tamanho do texto dentro do terminal.",
  termTheme: "Tema",
  termThemes: { dark: "Escuro", light: "Claro" },
  termFont: "Tipo de letra",
  termFontSize: "Tamanho",

  diag: "Diagnóstico",
  diagNote: "Informação útil para reportar um problema.",
  diagRun: "Atualizar",
  diagUnavailable: "O diagnóstico não está disponível nesta versão.",
  diagDaemon: "Daemon",
  diagProto: "protocolo",
  diagApp: "Aplicação",
  diagInstance: "instância",
  diagOffline: "sem ligação",
  diagData: "Pasta de dados",
  diagLag: "Registo de lag",
  diagMissing: "Ferramentas em falta",
  diagNoneMissing: "Nenhuma",

  // terminal
  attachFailed: "não foi possível ligar a esta sessão: ",
  findPlaceholder: "Procurar no terminal",
  findPrev: "Anterior",
  findNext: "Seguinte",
  findClose: "Fechar pesquisa",
  fileMissing: (p: string) => `O ficheiro ${p} não existe nesta worktree.`,

  // deletions: the notice names what went away; a failure names it too
  fileDeleted: (n: string) => `Ficheiro apagado: ${n}`,
  folderDeleted: (n: string) => `Pasta apagada: ${n}`,
  cardDeleted: (n: string) => `Cartão apagado: ${n}`,
  automationDeleted: (n: string) => `Automatização apagada: ${n}`,
  worktreeRemoved: (n: string) => `Worktree removido: ${n}`,
  projectRemoved: (n: string) => `Projeto tirado da lista: ${n}`,
  deleteFailed: (n: string, e: unknown) => `Não foi possível apagar "${n}": ${String(e).replace(/^Error: /, "")}`,
} as const;
