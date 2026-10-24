import { count, dict } from "./i18n";
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
    diagCopied: "Diagnóstico copiado para a área de transferência.",
  },
  actionHint: "ação",
  lang: { title: "Idioma", note: "Muda já, sem reiniciar. Fica guardado nas definições." },
  cmd: {
    palette: { label: "Paleta de comandos" },
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
    addRemote: { label: "Adicionar projeto remoto", hint: "anfitrião SSH ou WSL" },
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
    attention: "Parou: precisa de atenção",
    queued: "Na fila: o plano ainda não chegou a esta tarefa",
    running: "Terminal ativo",
    none: "",
  },
  session: (id: number) => `Sessão ${id}`,
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
    noDaemon: "Sem ligação ao daemon. Vai tentar de novo; se persistir, reinicia o Axelstra.",
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
    stopped: {
      head: (who: string, wt: string) => `${who} parou · ${wt}`,
      body: (who: string, wt: string) =>
        `${who} ficou parado na worktree "${wt}" sem terminar. Abre para ver o que aconteceu.`,
    },
    ship: {
      merged: (pr: string, wt: string) => `Merge feito: ${pr} (${wt})`,
      ciFail: (pr: string, wt: string) => `CI a falhar: ${pr} (${wt})`,
      ciPass: (pr: string, wt: string) => `CI a passar: ${pr} (${wt})`,
    },
    janitor: (branch: string, into: string) => `worktree ${branch} removido (integrado em ${into})`,
  },
  restore: {
    restored: (n: number) => count(n, "sessão restaurada", "sessões restauradas"),
    skipped: (n: number) => count(n, "conversa vazia ignorada", "conversas vazias ignoradas"),
  },

  // ---- voice dictation ----
  voice: {
    denied: "O acesso ao microfone foi recusado. Permite-o nas definições de privacidade do Windows.",
    missing: "Não foi encontrado nenhum microfone. Liga um e tenta de novo.",
    busy: "O microfone está a ser usado por outra aplicação.",
    failed: "Não foi possível usar o microfone. Tenta de novo.",
    noModel: "O modelo de ditado não está disponível. Descarrega-o de novo.",
    transcribeFailed: "A transcrição falhou. Tenta ditar outra vez.",
    downloadFailed: "Não foi possível descarregar o modelo de ditado. Verifica a ligação e tenta de novo.",
  },
} as const;

export const L = dict<typeof ptL>(ptL, enL, ptBRL);
