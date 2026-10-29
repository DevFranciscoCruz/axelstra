import { type Widen, count, fmtFixed } from "./i18n";
import type { ptL } from "./strings.logic";

/** Brazilian Portuguese texts built by the logic modules (commands and cheat sheet, status dots, friendly errors, notifications, ...), written out in full. */
export const ptBRL: Widen<typeof ptL> = {
  // ---- commands: the palette and the keyboard cheat sheet ----
  groups: {
    geral: "Geral",
    vista: "Visualização",
    separadores: "Abas e terminais",
    worktree: "Worktrees e projetos",
    ferramentas: "Ferramentas",
    rato: "Mouse",
  },
  about: {
    title: "Sobre",
    note: "Versão instalada e o daemon que está rodando. Copie ao reportar um problema.",
    app: "Aplicativo",
    daemon: "Daemon em execução",
    version: "Versão",
    commit: "Commit",
    built: "Compilado em",
    proto: "Protocolo",
    buildId: "Build do daemon",
    os: "Sistema",
    webview: "Componente web (WebView2)",
    unknown: "desconhecido",
    offline: "sem conexão",
    inSync: "O aplicativo e o daemon estão na mesma versão.",
    older: (daemon: string, app: string) =>
      `O daemon em execução (${daemon}) é mais antigo que o aplicativo (${app}). Reinicie-o para usar a versão nova; os terminais e agentes voltam sozinhos.`,
    newer: (daemon: string, app: string) =>
      `O daemon em execução (${daemon}) é mais recente que o aplicativo (${app}). Atualize o aplicativo.`,
    differs:
      "O aplicativo e o daemon têm o mesmo número de versão, mas foram compilados de commits diferentes. Reinicie o daemon.",
    noDaemon: "Sem conexão com o daemon: não dá para comparar as versões.",
    copy: "Copiar informações",
    copied: "Informações copiadas.",
    chipTip: (version: string, commit: string, built: string) =>
      `Axelstra ${version} · commit ${commit} · compilado em ${built}. Clique para ver os detalhes.`,
    chipLabel: "Sobre o Axelstra",
  },
  settingsRows: {
    a11y: "Acessibilidade",
    a11yNote: "Para leitores de tela. Desativado por padrão porque deixa os terminais mais lentos.",
    screenReader: "Modo leitor de tela nos terminais",
    backup: "Backup",
    backupNote: "Salva ou restaura as configurações, os projetos e as sessões do daemon.",
    export: "Exportar…",
    restore: "Restaurar…",
    exported: "Backup salvo em ",
    restored: "Backup restaurado. Reinicie o Axelstra para aplicar.",
    noBackup: "Este daemon ainda não suporta backups. Atualize e reinicie o Axelstra.",
    restorePath: "Escolha a pasta do backup a restaurar",
    copyDiag: "Copiar diagnóstico",
    cleanTitle: "Limpeza automática",
    cleanNote:
      "Remove sozinha o que já foi integrado: worktrees, branches ax/*, pastas órfãs, backups e registros antigos. O que tem alterações ou trabalho por integrar nunca é excluído.",
    cleanNow: "Limpar agora",
    cleanNothing: "Nada para limpar.",
    cleanRemoved: "Removido",
    cleanClosed: (sessions: number, procs: number, bytes: number) =>
      `${count(sessions, "sessão fechada", "sessões fechadas")} · ${count(procs, "processo encerrado", "processos encerrados")} · ${fmtFixed(bytes / 1048576, 0)} MB de RAM liberados`,
    cleanWarning: "Aviso",
    diagCopied: "Diagnóstico copiado para a área de transferência.",
  },
  actionHint: "ação",
  lang: { title: "Idioma", note: "Muda na hora, sem reiniciar. Fica salvo nas configurações." },
  cmd: {
    palette: { label: "Paleta de comandos (tudo o resto)" },
    settings: { label: "Configurações" },
    shortcuts: { label: "Atalhos de teclado" },
    contextMenu: { label: "Menu de contexto do elemento em foco" },
    confirm: { label: "Confirmar o diálogo, o cartão ou o commit" },
    toggleSidebar: { label: "Mostrar/ocultar projetos" },
    toggleRight: { label: "Mostrar/ocultar painel direito" },
    panelFiles: { label: "Painel: arquivos", hint: "painel" },
    panelChanges: { label: "Painel: alterações", hint: "painel" },
    panelPulls: { label: "Painel: PRs e problemas", hint: "painel" },
    docker: { label: "Docker", hint: "visualização" },
    kanban: { label: "Kanban", hint: "visualização" },
    notifications: { label: "Notificações", hint: "histórico de avisos" },
    theme: { label: "Mudar o tema", hint: "sistema → escuro → claro" },
    language: { label: "Mudar o idioma", hint: "português → inglês → português (Brasil)" },
    nextTab: { label: "Aba seguinte / anterior" },
    gotoTab: { label: "Ir para a aba n" },
    reopen: { label: "Reabrir a última aba fechada" },
    nextWaiting: { label: "Ir para o próximo agente aguardando resposta" },
    rename: { label: "Renomear a aba" },
    termFind: { label: "Pesquisar no terminal" },
    newAgent: { label: "Novo agente", hint: "agente" },
    newTerminal: { label: "Novo terminal", hint: "terminal" },
    split: { label: "Dividir terminal", hint: "visualização" },
    newWorktree: { label: "Novo worktree", hint: "worktree" },
    removeWorktree: { label: "Remover worktree atual", hint: "worktree" },
    searchChats: { label: "Pesquisar conversas" },
    voice: { label: "Ditado por voz" },
    orchestra: { label: "Orquestração: agentes e workers", hint: "ferramenta" },
    dispatcher: { label: "Despachante", hint: "pedidos enviados a cada worktree" },
    addProject: { label: "Adicionar projeto", hint: "pasta local ou remota" },
    addRemote: { label: "Adicionar projeto remoto", hint: "host SSH ou WSL" },
    automations: { label: "Automações", hint: "agendamento" },
    computer: { label: "Controle da tela pelos agentes", hint: "permissão" },
    restartDaemon: { label: "Reiniciar o daemon", hint: "fecha os terminais; os agentes são retomados" },
    tabRename: { label: "Renomear uma aba", keys: "Clique duplo na aba" },
    tabClose: { label: "Fechar uma aba", keys: "Clique com o botão do meio na aba" },
  },

  // ---- status dots, session names ----
  dot: {
    working: "Trabalhando",
    idle: "Parado",
    waiting: "Aguardando resposta",
    done: "Concluído",
    awaiting: "Aguardando algo externo",
    stalled: "Parado: sem reportar, precisa de atenção",
    attention: "Parou: precisa de atenção",
    queued: "Na fila: o plano ainda não chegou a esta tarefa",
    paused: "Pausada",
    hibernated: "Hibernada",
    running: "Terminal ativo",
    none: "",
  },
  awaitingTip: (reason: string, since: string, until: string | null) =>
    `Aguardando algo externo: ${reason} (${since})${until ? ` · expira ${until}` : ""}`,
  session: (id: number) => `Sessão ${id}`,
  inWorktree: (name: string) => `em ${name}`,
  sameFiles: (names: string) => `Edita os mesmos arquivos que: ${names}`,
  defaultAgent: "O agente",
  tabRenameFailed: "Não foi possível salvar o nome da aba: ",
  unsavedFile: "Há alterações não salvas neste arquivo.",
  editorTitle: "Editor",
  pickProject: "Escolha a pasta do projeto",
  unknownError: "Erro desconhecido",

  // ---- the daemon, git, gh and docker speak English: the common failures get a sentence that says what to do ----
  friendly: {
    notFound: "Programa não encontrado. Confirme se está instalado e no PATH (git, claude, codex ou gh).",
    exists: "Já existe um branch ou worktree com esse nome. Escolha outro nome.",
    notRepo: "Esta pasta não é um repositório git.",
    dirty: "Há alterações não salvas. Faça commit ou descarte-as primeiro.",
    remote: "Sem acesso ao remoto. Verifique a conexão com a internet e o login no git/gh.",
    rejected: "O remoto recusou o envio porque tem commits novos. Obtenha as alterações do remoto antes de enviar.",
    workerLimit: "Limite de agentes simultâneos atingido. Aguarde um terminar ou aumente com `axelstra limit`.",
    noDaemon: "Sem conexão com o daemon. Vamos tentar de novo; se persistir, reinicie o Axelstra.",
    daemonSilent: "O daemon não respondeu a tempo. Se persistir, reinicie o Axelstra.",
    oldDaemon: "O daemon em execução é antigo. Reinicie o Axelstra para atualizá-lo.",
    dockerDown: "O Docker não está rodando. Abra o Docker Desktop (ou inicie o serviço) e tente novamente.",
    dockerGone: "O Docker não tem mais esse recurso: ele foi excluído nesse meio-tempo. Atualize a lista.",
    portBusy: "A porta já está sendo usada por outro programa. Encerre esse programa ou escolha outra porta.",
    ghAuth: "O gh não está com a sessão iniciada. Rode `gh auth login` em um terminal.",
    conflicts: "Há conflitos de integração. Resolva os arquivos em conflito e faça commit.",
    nothingToCommit: "Não há alterações para fazer commit.",
    badRef: "O branch ou commit indicado não existe neste repositório.",
    rateLimit: "O GitHub limitou as requisições. Aguarde alguns minutos e tente novamente.",
  },

  editor: {
    binary: "Arquivo binário: não pode ser aberto no editor.",
    notUtf8: "O arquivo não é texto UTF-8: não pode ser aberto no editor.",
    tooLarge: "Arquivo grande demais para o editor.",
  },

  // ---- time and usage ----
  usage: {
    none: "Nenhum agente usou tokens nestes worktrees",
    head: (spent: string, cache: string) =>
      `Tokens dos agentes (Claude e Codex): ${spent} gastos · ${cache} de cache lido`,
    row: (name: string, spent: string, input: string, output: string, write: string, read: string) =>
      `${name}: ${spent} (entrada ${input} · saída ${output} · cache ${write}/${read})`,
  },

  // ---- agents ----
  agent: {
    notInstalled: (name: string) => `${name} (não instalado)`,
    unknownInList: (id: string) => `${id} (desconhecido)`,
    unknown: (id: string) => `Agente desconhecido: ${id}`,
    notFound: (name: string, install: string) => `${name} não encontrado: instale com ${install}`,
  },

  // ---- notifications (system and bell) ----
  notify: {
    done: {
      head: (who: string, wt: string) => `${who} terminou · ${wt}`,
      body: (who: string, wt: string) => `${who} concluiu a tarefa no worktree "${wt}". Abra para revisar o resultado.`,
    },
    waiting: {
      head: (who: string, wt: string) => `${who} precisa de você · ${wt}`,
      body: (who: string, wt: string) =>
        `${who} fez uma pergunta no worktree "${wt}" e precisa da sua resposta para continuar.`,
    },
    wait: {
      permission: (who: string, wt: string, text: string) => `${who} pede permissão em ${wt}: ${text}`,
      question: (who: string, wt: string, text: string) => `${who} tem uma pergunta em ${wt}: ${text}`,
      permissionGeneric: (who: string, wt: string) => `${who} pede permissão em ${wt} e espera a sua resposta.`,
      questionGeneric: (who: string, wt: string) => `${who} tem uma pergunta em ${wt} e espera a sua resposta.`,
    },
    stalledHead: (who: string, wt: string) => `${who} sem atividade e sem reportar · ${wt}`,
    exitCode: (who: string, code: number) => `${who} terminou com o código de saída ${code}.`,
    ship: {
      merged: (pr: string, wt: string) => `Merge feito: ${pr} (${wt})`,
      ciFail: (pr: string, wt: string) => `CI falhando: ${pr} (${wt})`,
      ciPass: (pr: string, wt: string) => `CI passando: ${pr} (${wt})`,
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
    denied: "O acesso ao microfone foi negado. Permita-o nas configurações de privacidade do sistema.",
    missing: "Nenhum microfone foi encontrado. Conecte um e tente novamente.",
    busy: "O microfone está sendo usado por outro aplicativo.",
    failed: "Não foi possível usar o microfone. Tente novamente.",
    noModel: "O modelo de ditado não está disponível. Baixe-o novamente.",
    transcribeFailed: "A transcrição falhou. Tente ditar outra vez.",
    downloadFailed: "Não foi possível baixar o modelo de ditado. Verifique a conexão e tente novamente.",
    downloadCorrupt: "O modelo de ditado baixado está corrompido. Tente baixar de novo.",
    downloadDisk: "Não foi possível salvar o modelo de ditado no disco. Verifique o espaço livre e tente novamente.",
  },
};
