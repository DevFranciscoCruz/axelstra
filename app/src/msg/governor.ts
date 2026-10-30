import type { Widen } from "../i18n";

/** Texts of the message codes raised by the memory governor (prefix `governor.`; see crates/daemon/src/governor.rs). */
export const pt = {
  "governor.bad_config": "Configuração do governador de recursos inválida: {detail}",
  "governor.admission_denied":
    "Não iniciado: a memória está {level} ({free_mb} MB livres). Os agentes esperam que recupere; os workers estão limitados a {max}.",
  "governor.kernel_pool_high": "Memória do kernel alta ({gb} GB): possível fuga de handles/driver; reiniciar liberta.",
  "governor.kernel_pool_filter":
    "Memória do kernel alta ({gb} GB); a etiqueta {tag} ({mb} MB) é a cache de nomes do gestor de filtros (antivírus e outros filtros de ficheiros).",
  "governor.kernel_pool_tokens":
    "Memória do kernel alta ({gb} GB); a etiqueta {tag} ({mb} MB) são tokens de segurança, que muitos processos lançados deixam.",
  "governor.kernel_pool_files":
    "Memória do kernel alta ({gb} GB); a etiqueta {tag} ({mb} MB) são objetos de ficheiros abertos.",
  "governor.kernel_pool_watchers":
    "Memória do kernel alta ({gb} GB); a etiqueta {tag} ({mb} MB) são notificações de alteração de pastas (vigias).",
  "governor.kernel_pool_ntfs":
    "Memória do kernel alta ({gb} GB); a etiqueta {tag} ({mb} MB) são estruturas de ficheiros do NTFS.",
  "governor.kernel_pool_mm": "Memória do kernel alta ({gb} GB); a etiqueta {tag} ({mb} MB) é do gestor de memória.",
  "governor.kernel_pool_io":
    "Memória do kernel alta ({gb} GB); a etiqueta {tag} ({mb} MB) são buffers de nomes de E/S.",
  "governor.kernel_pool_etw":
    "Memória do kernel alta ({gb} GB); a etiqueta {tag} ({mb} MB) são buffers de rastreio de eventos (ETW).",
  "governor.kernel_pool_tag":
    "Memória do kernel alta ({gb} GB); a maior etiqueta é {tag} ({mb} MB), dono desconhecido.",
  "governor.kernel_load_high":
    "CPU do kernel alta ({kernel_pct}%, {pages} páginas/s): um driver ou filtro (antivírus, indexador) pode estar a sobrecarregar o sistema.",
  "governor.pause_failed": "A sessão {id} não pôde ser pausada (já está pausada ou não há nada para parar).",
} as const satisfies Record<string, string>;

export const en: Widen<typeof pt> = {
  "governor.bad_config": "Invalid resource governor settings: {detail}",
  "governor.admission_denied":
    "Not started: memory is {level} ({free_mb} MB free). Agents wait until it recovers; workers are limited to {max}.",
  "governor.kernel_pool_high": "Kernel memory is high ({gb} GB): possible handle or driver leak; restarting frees it.",
  "governor.kernel_pool_filter":
    "Kernel memory is high ({gb} GB); the biggest pool tag {tag} ({mb} MB) is the file-name cache of the filter manager (antivirus and other file filters).",
  "governor.kernel_pool_tokens":
    "Kernel memory is high ({gb} GB); the biggest pool tag {tag} ({mb} MB) is security tokens, which many started processes leave.",
  "governor.kernel_pool_files":
    "Kernel memory is high ({gb} GB); the biggest pool tag {tag} ({mb} MB) is open file objects.",
  "governor.kernel_pool_watchers":
    "Kernel memory is high ({gb} GB); the biggest pool tag {tag} ({mb} MB) is folder change notifications (watchers).",
  "governor.kernel_pool_ntfs":
    "Kernel memory is high ({gb} GB); the biggest pool tag {tag} ({mb} MB) is NTFS file structures.",
  "governor.kernel_pool_mm":
    "Kernel memory is high ({gb} GB); the biggest pool tag {tag} ({mb} MB) belongs to the memory manager.",
  "governor.kernel_pool_io":
    "Kernel memory is high ({gb} GB); the biggest pool tag {tag} ({mb} MB) is I/O name buffers.",
  "governor.kernel_pool_etw":
    "Kernel memory is high ({gb} GB); the biggest pool tag {tag} ({mb} MB) is event tracing (ETW) buffers.",
  "governor.kernel_pool_tag":
    "Kernel memory is high ({gb} GB); the biggest pool tag is {tag} ({mb} MB), owner not known.",
  "governor.kernel_load_high":
    "Kernel CPU is high ({kernel_pct}%, {pages} pages/s): a driver or filter (antivirus, indexer) may be overloading the system.",
  "governor.pause_failed": "Session {id} could not be paused (already paused, or nothing to stop).",
};

export const ptBR: Widen<typeof pt> = {
  "governor.bad_config": "Configuração do governador de recursos inválida: {detail}",
  "governor.admission_denied":
    "Não iniciado: a memória está {level} ({free_mb} MB livres). Os agentes esperam ela se recuperar; os workers estão limitados a {max}.",
  "governor.kernel_pool_high":
    "Memória do kernel alta ({gb} GB): possível vazamento de handles/driver; reiniciar libera.",
  "governor.kernel_pool_filter":
    "Memória do kernel alta ({gb} GB); a etiqueta {tag} ({mb} MB) é o cache de nomes do gerenciador de filtros (antivírus e outros filtros de arquivos).",
  "governor.kernel_pool_tokens":
    "Memória do kernel alta ({gb} GB); a tag {tag} ({mb} MB) são tokens de segurança, que muitos processos iniciados deixam.",
  "governor.kernel_pool_files":
    "Memória do kernel alta ({gb} GB); a tag {tag} ({mb} MB) são objetos de arquivos abertos.",
  "governor.kernel_pool_watchers":
    "Memória do kernel alta ({gb} GB); a tag {tag} ({mb} MB) são notificações de mudança de pastas (vigias).",
  "governor.kernel_pool_ntfs":
    "Memória do kernel alta ({gb} GB); a tag {tag} ({mb} MB) são estruturas de arquivos do NTFS.",
  "governor.kernel_pool_mm": "Memória do kernel alta ({gb} GB); a tag {tag} ({mb} MB) é do gerenciador de memória.",
  "governor.kernel_pool_io": "Memória do kernel alta ({gb} GB); a tag {tag} ({mb} MB) são buffers de nomes de E/S.",
  "governor.kernel_pool_etw":
    "Memória do kernel alta ({gb} GB); a tag {tag} ({mb} MB) são buffers de rastreamento de eventos (ETW).",
  "governor.kernel_pool_tag": "Memória do kernel alta ({gb} GB); a maior tag é {tag} ({mb} MB), dono desconhecido.",
  "governor.kernel_load_high":
    "CPU do kernel alta ({kernel_pct}%, {pages} páginas/s): um driver ou filtro (antivírus, indexador) pode estar sobrecarregando o sistema.",
  "governor.pause_failed": "A sessão {id} não pôde ser pausada (já está pausada ou não há nada para parar).",
};
