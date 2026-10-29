import type { Widen } from "../i18n";

/** Texts of the message codes raised by the memory governor (prefix `governor.`; see crates/daemon/src/governor.rs). */
export const pt = {
  "governor.bad_config": "Configuração do governador de recursos inválida: {detail}",
  "governor.admission_denied":
    "Não iniciado: a memória está {level} ({free_mb} MB livres). Os agentes esperam que recupere; os workers estão limitados a {max}.",
  "governor.kernel_pool_high": "Memória do kernel alta ({gb} GB): possível fuga de handles/driver; reiniciar liberta.",
  "governor.kernel_load_high":
    "CPU do kernel alta ({kernel_pct}%, {pages} páginas/s): um driver ou filtro (antivírus, indexador) pode estar a sobrecarregar o sistema.",
  "governor.pause_failed": "A sessão {id} não pôde ser pausada (já está pausada ou não há nada para parar).",
} as const satisfies Record<string, string>;

export const en: Widen<typeof pt> = {
  "governor.bad_config": "Invalid resource governor settings: {detail}",
  "governor.admission_denied":
    "Not started: memory is {level} ({free_mb} MB free). Agents wait until it recovers; workers are limited to {max}.",
  "governor.kernel_pool_high": "Kernel memory is high ({gb} GB): possible handle or driver leak; restarting frees it.",
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
  "governor.kernel_load_high":
    "CPU do kernel alta ({kernel_pct}%, {pages} páginas/s): um driver ou filtro (antivírus, indexador) pode estar sobrecarregando o sistema.",
  "governor.pause_failed": "A sessão {id} não pôde ser pausada (já está pausada ou não há nada para parar).",
};
