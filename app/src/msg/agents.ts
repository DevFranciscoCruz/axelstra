import type { Widen } from "../i18n";

/** Texts of the message codes raised by the daemon (see crates/core/src/msg.rs). `{name}` is filled from the call's params.
 *  pt-PT is the source; `en` and `ptBR` must have exactly the same keys (compile error otherwise, `msg.test.ts` repeats it). */
export const pt = {
  // agents and sessions
  "agent.unknown": "Agente desconhecido '{id}'. Usa um destes: {ids}.",
  "agent.not_installed": "{name} não foi encontrado. Instala-o com {install}.",
  "agent.bad_status_key": "Chave inválida.",
  "session.not_found": "Não existe nenhuma sessão {id}.",
  "session.worker_limit":
    "Limite de workers atingido ({live} a correr, máximo {max}). Espera que um acabe ou aumenta o limite com `axelstra limit N`.",
  "session.bad_state": "O estado tem de ser waiting, idle ou working.",
  "session.input_full": "A fila de entrada da sessão está cheia: o programa não está a ler o que lhe enviam.",
  "session.input_closed": "A entrada da sessão está fechada.",
  "session.spawn_failed": "Não foi possível iniciar {program}.\n({detail})",
  "msg.unknown_kind": "Tipo de mensagem desconhecido: {kind}.",

  // task files
  "notes.too_many_files": "Há ficheiros de tarefa a mais com o nome {base}.",
  "notes.empty_text": "Não há nada para escrever.",
  "notes.text_too_long": "O texto é demasiado longo (máximo {kb} KB).",
  "notes.file_full": "O ficheiro de tarefa está cheio.",
  "notes.too_large": "O ficheiro de tarefa é grande demais para ser mostrado.",
  "notes.bad_file": "Ficheiro de tarefa inválido.",
  "notes.link_dir": "{path} é um atalho (link): não se exporta através dele.",
  "notes.link_target": "{path} é um atalho (link): não se escreve por cima dele.",
  "notes.exists": "{path} já existe. Exporta outra vez com a opção de substituir.",
  "notes.not_found": "Ficheiro de tarefa não encontrado.",
  "notes.session_without_file": "Esta sessão não tem ficheiro de tarefa.",

  // hooks of the agents
  "hooks.unsupported_agent": "Só o claude e o gemini têm hooks.",
  "hooks.read_failed": "Não foi possível ler {path}.\n({detail})",
  "hooks.not_json": "{path} não é JSON válido ({detail}). Não lhe toquei: corrige-o à mão e tenta outra vez.",
  "hooks.not_object": "{path} não é um objeto JSON. Não lhe toquei.",
  "hooks.hooks_not_object": '"hooks" em {path} não é um objeto. Não lhe toquei.',
  "hooks.event_not_list": '"hooks.{event}" em {path} não é uma lista. Não lhe toquei.',
  "hooks.backup_failed": "Não foi possível escrever a cópia de segurança {path}.\n({detail})",
  "hooks.write_failed": "Não foi possível escrever {path}.\n({detail})",

  // budgets
  "budget.not_object": "Os limites ({what}) têm de ser um objeto.",
  "budget.unknown_key": "Limites ({what}): chave desconhecida {k}.",
  "budget.bad_limit": "Limites ({what}): {k} tem de ser um número inteiro de tokens entre 1 e {max}, ou nulo.",
  "budget.bad_auto_pause": "Limites ({what}): auto_pause tem de ser verdadeiro ou falso.",
  "budget.root_not_object": "Os limites têm de ser um objeto.",
  "budget.root_unknown_key": "Limites: chave desconhecida {k}.",
  "budget.worktrees_not_object": "Os limites por worktree têm de ser um objeto.",
  "budget.too_many_worktrees": "Os limites por worktree têm, no máximo, {max} entradas.",
  "budget.bad_path": "Limites por worktree: o caminho tem de ter de 1 a 1024 caracteres.",

  // triggers (CI fix)
  "triggers.bad_number": "{k} tem de ser um número inteiro de {lo} a {hi}.",
  "triggers.bad_ci_fix": "ci_fix tem de ser verdadeiro ou falso.",
  "triggers.confirm_needed": 'Para ativar pela primeira vez, confirma escrevendo "{word}".',
  "triggers.cooldown": "Limitado: pausa do worktree, faltam {secs} s.",
  "triggers.hourly_max": "Limitado: máximo por hora do projeto, faltam {secs} s.",

  // settings
  "settings.bad_shell": "A shell tem de ser auto, pwsh, powershell, bash, wsl ou cmd.",
  "settings.bad_notify_mode": "O modo de notificações tem de ser all, waiting ou off.",
  "settings.bad_stall_minutes": "Os minutos de inatividade têm de ser um número inteiro de 0 (desligado) a 1440.",
  "agent.stalled": "{task}: sem atividade há {min} min e sem reportar. Última linha: {line}",
  "settings.bad_terse": "O modo conciso tem de ser off, short ou tiny.",
  "settings.bad_pm": "O gestor de pacotes tem de ser bun, pnpm, yarn ou npm.",
  "settings.ui_not_object": "O estado da interface tem de ser um objeto de textos.",
  "settings.ui_key_not_allowed": "A chave {k} da interface não é permitida.",
  "settings.ui_bad_value": "O valor de {k} tem de ser um texto de, no máximo, {max} bytes.",
  "settings.ui_too_large": "O estado da interface é demasiado grande ({keys} chaves, {total} bytes).",

  // raised by the dispatch files of this group
  "automation.unknown": "Não existe nenhuma automação {id}.",
  "workspace.bad_mode": "O modo tem de ser merge ou replace, não {m}.",
  "workspace.bad_conflicts": "Os conflitos têm de ser keep ou overwrite, não {m}.",
  "worktree.pr_env_not_copied":
    "{count} ficheiro(s) ignorado(s) do .worktreeinclude ({files}) NÃO foram copiados para o worktree desta pull request, porque o código dela não é de confiança. Copia-os à mão se precisares deles.",
} as const satisfies Record<string, string>;

export const en: Widen<typeof pt> = {
  "agent.unknown": "Unknown agent '{id}'. Use one of: {ids}.",
  "agent.not_installed": "{name} was not found. Install it with {install}.",
  "agent.bad_status_key": "Invalid key.",
  "session.not_found": "There is no session {id}.",
  "session.worker_limit":
    "Worker limit reached ({live} running, max {max}). Wait for one to finish or raise the limit with `axelstra limit N`.",
  "session.bad_state": "The state must be waiting, idle or working.",
  "session.input_full": "The session's input queue is full: the program is not reading what it is sent.",
  "session.input_closed": "The session's input is closed.",
  "session.spawn_failed": "Could not start {program}.\n({detail})",
  "msg.unknown_kind": "Unknown message kind: {kind}.",

  "notes.too_many_files": "There are too many task files named {base}.",
  "notes.empty_text": "There is nothing to write.",
  "notes.text_too_long": "The text is too long (max {kb} KB).",
  "notes.file_full": "The task file is full.",
  "notes.too_large": "The task file is too large to show.",
  "notes.bad_file": "Invalid task file.",
  "notes.link_dir": "{path} is a link: refusing to export through it.",
  "notes.link_target": "{path} is a link: refusing to overwrite it.",
  "notes.exists": "{path} already exists. Export again with the overwrite option.",
  "notes.not_found": "Task file not found.",
  "notes.session_without_file": "This session has no task file.",

  "hooks.unsupported_agent": "Only claude and gemini have hooks.",
  "hooks.read_failed": "Could not read {path}.\n({detail})",
  "hooks.not_json": "{path} is not valid JSON ({detail}). I did not touch it: fix it by hand and try again.",
  "hooks.not_object": "{path} is not a JSON object. I did not touch it.",
  "hooks.hooks_not_object": '"hooks" in {path} is not an object. I did not touch it.',
  "hooks.event_not_list": '"hooks.{event}" in {path} is not a list. I did not touch it.',
  "hooks.backup_failed": "Could not write the backup copy {path}.\n({detail})",
  "hooks.write_failed": "Could not write {path}.\n({detail})",

  "budget.not_object": "The budgets ({what}) must be an object.",
  "budget.unknown_key": "Budgets ({what}): unknown key {k}.",
  "budget.bad_limit": "Budgets ({what}): {k} must be a whole number of tokens between 1 and {max}, or null.",
  "budget.bad_auto_pause": "Budgets ({what}): auto_pause must be true or false.",
  "budget.root_not_object": "The budgets must be an object.",
  "budget.root_unknown_key": "Budgets: unknown key {k}.",
  "budget.worktrees_not_object": "The per-worktree budgets must be an object.",
  "budget.too_many_worktrees": "The per-worktree budgets hold at most {max} entries.",
  "budget.bad_path": "Per-worktree budgets: a path must have 1 to 1024 characters.",

  "triggers.bad_number": "{k} must be a whole number from {lo} to {hi}.",
  "triggers.bad_ci_fix": "ci_fix must be true or false.",
  "triggers.confirm_needed": 'To enable it for the first time, confirm by typing "{word}".',
  "triggers.cooldown": "Rate limited: worktree cooldown, {secs}s left.",
  "triggers.hourly_max": "Rate limited: project hourly maximum, {secs}s left.",

  "settings.bad_shell": "The shell must be auto, pwsh, powershell, bash, wsl or cmd.",
  "settings.bad_notify_mode": "The notification mode must be all, waiting or off.",
  "settings.bad_stall_minutes": "The inactivity minutes must be a whole number from 0 (off) to 1440.",
  "agent.stalled": "{task} has been stalled for {min} min without reporting. Last line: {line}",
  "settings.bad_terse": "The terse mode must be off, short or tiny.",
  "settings.bad_pm": "The package manager must be bun, pnpm, yarn or npm.",
  "settings.ui_not_object": "The interface state must be an object of strings.",
  "settings.ui_key_not_allowed": "The interface key {k} is not allowed.",
  "settings.ui_bad_value": "The value of {k} must be a string of at most {max} bytes.",
  "settings.ui_too_large": "The interface state is too large ({keys} keys, {total} bytes).",

  "automation.unknown": "There is no automation {id}.",
  "workspace.bad_mode": "The mode must be merge or replace, not {m}.",
  "workspace.bad_conflicts": "The conflicts option must be keep or overwrite, not {m}.",
  "worktree.pr_env_not_copied":
    "{count} ignored file(s) from .worktreeinclude ({files}) were NOT copied into this pull request's worktree, because its code is not trusted. Copy them by hand if you need them.",
};

/** Brazilian wording: written out in full, not a fallback to pt-PT. */
export const ptBR: Widen<typeof pt> = {
  "agent.unknown": "Agente desconhecido '{id}'. Use um destes: {ids}.",
  "agent.not_installed": "{name} não foi encontrado. Instale-o com {install}.",
  "agent.bad_status_key": "Chave inválida.",
  "session.not_found": "Não existe nenhuma sessão {id}.",
  "session.worker_limit":
    "Limite de workers atingido ({live} rodando, máximo {max}). Espere um terminar ou aumente o limite com `axelstra limit N`.",
  "session.bad_state": "O estado precisa ser waiting, idle ou working.",
  "session.input_full": "A fila de entrada da sessão está cheia: o programa não está lendo o que é enviado a ele.",
  "session.input_closed": "A entrada da sessão está fechada.",
  "session.spawn_failed": "Não foi possível iniciar {program}.\n({detail})",
  "msg.unknown_kind": "Tipo de mensagem desconhecido: {kind}.",

  "notes.too_many_files": "Há arquivos de tarefa demais com o nome {base}.",
  "notes.empty_text": "Não há nada para escrever.",
  "notes.text_too_long": "O texto é longo demais (máximo {kb} KB).",
  "notes.file_full": "O arquivo de tarefa está cheio.",
  "notes.too_large": "O arquivo de tarefa é grande demais para ser exibido.",
  "notes.bad_file": "Arquivo de tarefa inválido.",
  "notes.link_dir": "{path} é um link: não se exporta através dele.",
  "notes.link_target": "{path} é um link: não se escreve por cima dele.",
  "notes.exists": "{path} já existe. Exporte de novo com a opção de substituir.",
  "notes.not_found": "Arquivo de tarefa não encontrado.",
  "notes.session_without_file": "Esta sessão não tem arquivo de tarefa.",

  "hooks.unsupported_agent": "Só o claude e o gemini têm hooks.",
  "hooks.read_failed": "Não foi possível ler {path}.\n({detail})",
  "hooks.not_json": "{path} não é um JSON válido ({detail}). Não mexi nele: corrija à mão e tente de novo.",
  "hooks.not_object": "{path} não é um objeto JSON. Não mexi nele.",
  "hooks.hooks_not_object": '"hooks" em {path} não é um objeto. Não mexi nele.',
  "hooks.event_not_list": '"hooks.{event}" em {path} não é uma lista. Não mexi nele.',
  "hooks.backup_failed": "Não foi possível salvar o backup {path}.\n({detail})",
  "hooks.write_failed": "Não foi possível salvar {path}.\n({detail})",

  "budget.not_object": "Os limites ({what}) precisam ser um objeto.",
  "budget.unknown_key": "Limites ({what}): chave desconhecida {k}.",
  "budget.bad_limit": "Limites ({what}): {k} precisa ser um número inteiro de tokens entre 1 e {max}, ou nulo.",
  "budget.bad_auto_pause": "Limites ({what}): auto_pause precisa ser verdadeiro ou falso.",
  "budget.root_not_object": "Os limites precisam ser um objeto.",
  "budget.root_unknown_key": "Limites: chave desconhecida {k}.",
  "budget.worktrees_not_object": "Os limites por worktree precisam ser um objeto.",
  "budget.too_many_worktrees": "Os limites por worktree têm, no máximo, {max} entradas.",
  "budget.bad_path": "Limites por worktree: o caminho precisa ter de 1 a 1024 caracteres.",

  "triggers.bad_number": "{k} precisa ser um número inteiro de {lo} a {hi}.",
  "triggers.bad_ci_fix": "ci_fix precisa ser verdadeiro ou falso.",
  "triggers.confirm_needed": 'Para ativar pela primeira vez, confirme digitando "{word}".',
  "triggers.cooldown": "Limitado: pausa do worktree, faltam {secs} s.",
  "triggers.hourly_max": "Limitado: máximo por hora do projeto, faltam {secs} s.",

  "settings.bad_shell": "O shell precisa ser auto, pwsh, powershell, bash, wsl ou cmd.",
  "settings.bad_notify_mode": "O modo de notificações precisa ser all, waiting ou off.",
  "settings.bad_stall_minutes": "Os minutos de inatividade precisam ser um número inteiro de 0 (desligado) a 1440.",
  "agent.stalled": "{task}: sem atividade há {min} min e sem reportar. Última linha: {line}",
  "settings.bad_terse": "O modo conciso precisa ser off, short ou tiny.",
  "settings.bad_pm": "O gerenciador de pacotes precisa ser bun, pnpm, yarn ou npm.",
  "settings.ui_not_object": "O estado da interface precisa ser um objeto de textos.",
  "settings.ui_key_not_allowed": "A chave {k} da interface não é permitida.",
  "settings.ui_bad_value": "O valor de {k} precisa ser um texto de, no máximo, {max} bytes.",
  "settings.ui_too_large": "O estado da interface é grande demais ({keys} chaves, {total} bytes).",

  "automation.unknown": "Não existe nenhuma automação {id}.",
  "workspace.bad_mode": "O modo precisa ser merge ou replace, não {m}.",
  "workspace.bad_conflicts": "Os conflitos precisam ser keep ou overwrite, não {m}.",
  "worktree.pr_env_not_copied":
    "{count} arquivo(s) ignorado(s) do .worktreeinclude ({files}) NÃO foram copiados para o worktree desta pull request, porque o código dela não é confiável. Copie-os à mão se precisar deles.",
};
