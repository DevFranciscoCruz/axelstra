import { count, dict, fmtFixed } from "./i18n";
import { enL } from "./strings.logic.en";
import { ptBRL } from "./strings.logic.ptbr";

/** pt-PT texts built by the logic modules (commands and cheat sheet, status dots, friendly errors, notifications, ...). */
export const ptL = {
  // ---- commands: the palette and the keyboard cheat sheet ----
  groups: {
    geral: "Geral",
    vista: "Vista",
    separadores: "Separadores e terminais",
    worktree: "Worktrees e projetos",
    ferramentas: "Ferramentas",
    rato: "Rato",
  },
  about: {
    title: "Acerca",
    note: "Versão instalada e o daemon que está a correr. Copia-a ao reportar um problema.",
    app: "Aplicação",
    daemon: "Daemon em execução",
    version: "Versão",
    commit: "Commit",
    built: "Compilado em",
    proto: "Protocolo",
    buildId: "Build do daemon",
    os: "Sistema",
    webview: "Componente web (WebView2)",
    unknown: "desconhecido",
    offline: "sem ligação",
    inSync: "A aplicação e o daemon estão na mesma versão.",
    older: (daemon: string, app: string) =>
      `O daemon em execução (${daemon}) é mais antigo do que a aplicação (${app}). Reinicia-o para usar a versão nova; os terminais e agentes voltam sozinhos.`,
    newer: (daemon: string, app: string) =>
      `O daemon em execução (${daemon}) é mais recente do que a aplicação (${app}). Atualiza a aplicação.`,
    differs:
      "A aplicação e o daemon têm o mesmo número de versão mas foram compilados de commits diferentes. Reinicia o daemon.",
    noDaemon: "Sem ligação ao daemon: não é possível comparar as versões.",
    copy: "Copiar informação",
    copied: "Informação copiada.",
    chipTip: (version: string, commit: string, built: string) =>
      `Axelstra ${version} · commit ${commit} · compilado em ${built}. Clica para ver os detalhes.`,
    chipLabel: "Acerca do Axelstra",
  },
  settingsRows: {
    a11y: "Acessibilidade",
    a11yNote: "Para leitores de ecrã. Desligado por omissão porque deixa os terminais mais lentos.",
    screenReader: "Modo leitor de ecrã nos terminais",
    backup: "Cópia de segurança",
    backupNote: "Guarda ou repõe as definições, projetos e sessões do daemon.",
    export: "Exportar…",
    restore: "Restaurar…",
    exported: "Cópia guardada em ",
    restored: "Cópia restaurada. Reinicia o Axelstra para aplicar.",
    noBackup: "Este daemon ainda não suporta cópias de segurança. Atualiza e reinicia o Axelstra.",
    restorePath: "Escolhe a pasta da cópia a restaurar",
    copyDiag: "Copiar diagnóstico",
    cleanTitle: "Limpeza automática",
    cleanNote:
      "Remove sozinha o que já está integrado: worktrees, branches ax/*, pastas órfãs, cópias e registos antigos. O que tem alterações ou trabalho por integrar nunca se apaga.",
    cleanNow: "Limpar agora",
    cleanNothing: "Nada para limpar.",
    cleanRemoved: "Removido",
    cleanClosed: (sessions: number, procs: number, bytes: number) =>
      `${count(sessions, "sessão fechada", "sessões fechadas")} · ${count(procs, "processo terminado", "processos terminados")} · ${fmtFixed(bytes / 1048576, 0)} MB de RAM libertados`,
    cleanWarning: "Aviso",
    diagCopied: "Diagnóstico copiado para a área de transferência.",
  },
  actionHint: "ação",
  lang: { title: "Idioma", note: "Muda já, sem reiniciar. Fica guardado nas definições." },
  // ---- the settings page: side navigation, search ----
  settingsNav: {
    back: "Voltar à aplicação",
    search: "Pesquisar definições",
    navLabel: "Secções das definições",
    sections: "Secções",
    noMatch: "Nenhuma definição encontrada.",
    groups: {
      general: "Geral",
      performance: "Desempenho",
      integrations: "Ligações",
      maintenance: "Manutenção",
    },
    items: {
      general: {
        title: "Geral",
        sub: "Respostas dos agentes, gestor de pacotes, shell e deteção de agentes parados.",
        kw: "terse respostas gestor pacotes npm pnpm shell powershell bash parado stall",
      },
      language: { title: "Idioma", sub: "A língua da aplicação.", kw: "língua português english pt en" },
      appearance: {
        title: "Aparência",
        sub: "Tema da aplicação, terminal e acessibilidade.",
        kw: "tema escuro claro fonte letra terminal leitor ecrã acessibilidade",
      },
      notifications: {
        title: "Notificações",
        sub: "O que avisa, quando e com que som.",
        kw: "avisos som não incomodar dnd",
      },
      memory: {
        title: "Memória",
        sub: "Pressão de memória, governor e pausa de sessões.",
        kw: "governor ram pausa limite",
      },
      hibernate: {
        title: "Hibernação",
        sub: "Liberta a memória dos agentes parados e acorda-os quando for preciso.",
        kw: "dormir acordar ram inativo",
      },
      build: {
        title: "Build",
        sub: "Ambiente de compilação das sessões novas.",
        kw: "cargo jobs sccache cache target compilar",
      },
      budgets: { title: "Orçamentos", sub: "Limites de gasto dos agentes.", kw: "custo tokens limite gasto" },
      connections: { title: "Máquinas", sub: "Máquinas remotas e ligações.", kw: "ssh wsl remoto host servidor" },
      workspace: {
        title: "Espaço de trabalho",
        sub: "Exportar e importar o espaço de trabalho.",
        kw: "exportar importar projetos",
      },
      hooks: { title: "Hooks", sub: "Hooks dos agentes que o Axelstra instala.", kw: "claude gemini codex hook" },
      maintenance: {
        title: "Manutenção",
        sub: "Limpeza automática e cópias de segurança.",
        kw: "limpar worktrees backup cópia restaurar exportar",
      },
      diagnostics: {
        title: "Diagnóstico",
        sub: "Versões, pastas de dados e ferramentas em falta.",
        kw: "logs lag ferramentas dados",
      },
      about: { title: "Acerca", sub: "Versão da aplicação e do daemon.", kw: "versão commit atualização" },
    },
  },
  cmd: {
    palette: { label: "Paleta de comandos (tudo o resto)" },
    settings: { label: "Definições" },
    shortcuts: { label: "Atalhos de teclado" },
    contextMenu: { label: "Menu de contexto do elemento focado" },
    confirm: { label: "Confirmar o diálogo, o cartão ou o commit" },
    toggleSidebar: { label: "Mostrar/ocultar projetos" },
    toggleRight: { label: "Mostrar/ocultar painel direito" },
    panelFiles: { label: "Painel: ficheiros", hint: "painel" },
    panelChanges: { label: "Painel: alterações", hint: "painel" },
    panelPulls: { label: "Painel: PRs e problemas", hint: "painel" },
    docker: { label: "Docker", hint: "vista" },
    kanban: { label: "Kanban", hint: "vista" },
    notifications: { label: "Notificações", hint: "histórico de avisos" },
    theme: { label: "Mudar o tema", hint: "sistema → escuro → claro" },
    language: { label: "Mudar o idioma", hint: "português → inglês → português (Brasil)" },
    nextTab: { label: "Separador seguinte / anterior" },
    gotoTab: { label: "Ir para o separador n" },
    reopen: { label: "Reabrir o último separador fechado" },
    nextWaiting: { label: "Ir para o próximo agente à espera de resposta" },
    rename: { label: "Mudar o nome do separador" },
    termFind: { label: "Procurar no terminal" },
    newAgent: { label: "Novo agente", hint: "agente" },
    newTerminal: { label: "Novo terminal", hint: "terminal" },
    split: { label: "Dividir terminal", hint: "vista" },
    newWorktree: { label: "Novo worktree", hint: "worktree" },
    removeWorktree: { label: "Remover worktree atual", hint: "worktree" },
    searchChats: { label: "Pesquisar conversas" },
    voice: { label: "Ditado por voz" },
    orchestra: { label: "Orquestração: agentes e workers", hint: "ferramenta" },
    dispatcher: { label: "Despachante", hint: "pedidos enviados a cada worktree" },
    addProject: { label: "Adicionar projeto", hint: "pasta local ou remota" },
    addRemote: { label: "Adicionar projeto remoto", hint: "host SSH ou WSL" },
    automations: { label: "Automatizações", hint: "agendamento" },
    computer: { label: "Controlo do ecrã pelos agentes", hint: "permissão" },
    restartDaemon: { label: "Reiniciar o daemon", hint: "fecha os terminais; os agentes retomam" },
    tabRename: { label: "Mudar o nome de um separador", keys: "Duplo clique no separador" },
    tabClose: { label: "Fechar um separador", keys: "Clique do meio no separador" },
  },

  // ---- status dots, session names ----
  dot: {
    working: "A trabalhar",
    idle: "Parado",
    waiting: "À espera de resposta",
    done: "Terminado",
    awaiting: "À espera de algo externo",
    stalled: "Parada: sem reportar, precisa de atenção",
    attention: "Parou: precisa de atenção",
    queued: "Na fila: o plano ainda não chegou a esta tarefa",
    paused: "Em pausa",
    hibernated: "Hibernada",
    running: "Terminal ativo",
    none: "",
  },
  awaitingTip: (reason: string, since: string, until: string | null) =>
    `À espera de algo externo: ${reason} (${since})${until ? ` · expira ${until}` : ""}`,
  session: (id: number) => `Sessão ${id}`,
  inWorktree: (name: string) => `em ${name}`,
  sameFiles: (names: string) => `Edita os mesmos ficheiros que: ${names}`,
  defaultAgent: "O agente",
  tabRenameFailed: "Não foi possível guardar o nome do separador: ",
  unsavedFile: "Há alterações por guardar neste ficheiro.",
  editorTitle: "Editor",
  pickProject: "Escolhe a pasta do projeto",
  unknownError: "Erro desconhecido",

  // ---- the daemon, git, gh and docker speak English: the common failures get a sentence that says what to do ----
  friendly: {
    notFound: "Programa não encontrado. Confirma que está instalado e no PATH (git, claude, codex ou gh).",
    exists: "Já existe um branch ou worktree com esse nome. Escolhe outro nome.",
    notRepo: "Esta pasta não é um repositório git.",
    dirty: "Há alterações por guardar. Faz commit ou descarta-as primeiro.",
    remote: "Sem acesso ao remoto. Verifica a ligação à internet e o login no git/gh.",
    rejected: "O remoto recusou o envio porque tem commits novos. Obtém as alterações do remoto antes de enviar.",
    workerLimit: "Limite de agentes em simultâneo atingido. Espera que um termine ou aumenta com `axelstra limit`.",
    noDaemon: "Sem ligação ao daemon. O Axelstra vai tentar de novo; se persistir, reinicia-o.",
    daemonSilent: "O daemon não respondeu a tempo. Se persistir, reinicia o Axelstra.",
    oldDaemon: "O daemon em execução é antigo. Reinicia o Axelstra para o atualizar.",
    dockerDown: "O Docker não está a correr. Abre o Docker Desktop (ou inicia o serviço) e tenta de novo.",
    dockerGone: "O Docker já não tem esse recurso: foi apagado entretanto. Atualiza a lista.",
    portBusy: "A porta já está a ser usada por outro programa. Para esse programa ou escolhe outra porta.",
    ghAuth: "O gh não tem sessão iniciada. Corre `gh auth login` num terminal.",
    conflicts: "Há conflitos de integração. Resolve os ficheiros em conflito e faz commit.",
    nothingToCommit: "Não há alterações para fazer commit.",
    badRef: "O branch ou commit indicado não existe neste repositório.",
    rateLimit: "O GitHub limitou os pedidos. Espera uns minutos e tenta de novo.",
  },

  editor: {
    binary: "Ficheiro binário: não pode ser aberto no editor.",
    notUtf8: "O ficheiro não é texto UTF-8: não pode ser aberto no editor.",
    tooLarge: "Ficheiro demasiado grande para o editor.",
  },

  // ---- time and usage ----
  usage: {
    none: "Nenhum agente usou tokens nestes worktrees",
    head: (spent: string, cache: string) =>
      `Tokens dos agentes (Claude e Codex): ${spent} gastos · ${cache} de cache lida`,
    row: (name: string, spent: string, input: string, output: string, write: string, read: string) =>
      `${name}: ${spent} (entrada ${input} · saída ${output} · cache ${write}/${read})`,
  },

  // ---- agents ----
  agent: {
    notInstalled: (name: string) => `${name} (não instalado)`,
    unknownInList: (id: string) => `${id} (desconhecido)`,
    unknown: (id: string) => `Agente desconhecido: ${id}`,
    notFound: (name: string, install: string) => `${name} não encontrado: instala com ${install}`,
  },

  // ---- notifications (system and bell) ----
  notify: {
    done: {
      head: (who: string, wt: string) => `${who} terminou · ${wt}`,
      body: (who: string, wt: string) => `${who} acabou a tarefa na worktree "${wt}". Abre para rever o resultado.`,
    },
    waiting: {
      head: (who: string, wt: string) => `${who} precisa de ti · ${wt}`,
      body: (who: string, wt: string) =>
        `${who} fez uma pergunta na worktree "${wt}" e precisa da tua resposta para continuar.`,
    },
    wait: {
      permission: (who: string, wt: string, text: string) => `${who} pede permissão em ${wt}: ${text}`,
      question: (who: string, wt: string, text: string) => `${who} tem uma pergunta em ${wt}: ${text}`,
      permissionGeneric: (who: string, wt: string) => `${who} pede permissão em ${wt} e espera a tua resposta.`,
      questionGeneric: (who: string, wt: string) => `${who} tem uma pergunta em ${wt} e espera a tua resposta.`,
    },
    stalledHead: (who: string, wt: string) => `${who} sem atividade e sem reportar · ${wt}`,
    exitCode: (who: string, code: number) => `${who} terminou com o código de saída ${code}.`,
    ship: {
      merged: (pr: string, wt: string) => `Merge feito: ${pr} (${wt})`,
      ciFail: (pr: string, wt: string) => `CI a falhar: ${pr} (${wt})`,
      ciPass: (pr: string, wt: string) => `CI a passar: ${pr} (${wt})`,
    },
    janitor: (branch: string, into: string) => `worktree ${branch} removido (integrado em ${into})`,
    integrated: (branch: string, into: string) => `worktree ${branch} integrado em ${into} e removido`,
    emptyRemoved: (branch: string) => `Worktree vazio ${branch} removido`,
    agg: {
      done: (n: number) => `${count(n, "sessão concluiu", "sessões concluíram")} a tarefa`,
      merged: (n: number) => `${count(n, "worktree integrado", "worktrees integrados")} e removidos`,
    },
  },
  restore: {
    restored: (n: number) => count(n, "sessão restaurada", "sessões restauradas"),
    skipped: (n: number) => count(n, "conversa vazia ignorada", "conversas vazias ignoradas"),
  },

  // ---- voice dictation ----
  voice: {
    denied: "O acesso ao microfone foi recusado. Permite-o nas definições de privacidade do sistema.",
    missing: "Não foi encontrado nenhum microfone. Liga um e tenta de novo.",
    busy: "O microfone está a ser usado por outra aplicação.",
    failed: "Não foi possível usar o microfone. Tenta de novo.",
    noModel: "O modelo de ditado não está disponível. Descarrega-o de novo.",
    transcribeFailed: "A transcrição falhou. Tenta ditar outra vez.",
    downloadFailed: "Não foi possível descarregar o modelo de ditado. Verifica a ligação e tenta de novo.",
    downloadCorrupt: "O modelo de ditado descarregado está corrompido. Tenta descarregar de novo.",
    downloadDisk: "Não foi possível guardar o modelo de ditado no disco. Verifica o espaço livre e tenta de novo.",
  },
} as const;

export const L = dict<typeof ptL>(ptL, enL, ptBRL);
