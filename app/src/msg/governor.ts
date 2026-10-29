import type { Widen } from "../i18n";

/** Texts of the message codes raised by the memory governor (prefix `governor.`; see crates/daemon/src/governor.rs). */
export const pt = {
  "governor.bad_config": "Configuração do governador de recursos inválida: {detail}",
  "governor.admission_denied":
    "Não iniciado: a memória está {level} ({free_mb} MB livres). Os agentes esperam que recupere; os workers estão limitados a {max}.",
  "governor.pause_failed": "A sessão {id} não pôde ser pausada (já está pausada ou não há nada para parar).",
} as const satisfies Record<string, string>;

export const en: Widen<typeof pt> = {
  "governor.bad_config": "Invalid resource governor settings: {detail}",
  "governor.admission_denied":
    "Not started: memory is {level} ({free_mb} MB free). Agents wait until it recovers; workers are limited to {max}.",
  "governor.pause_failed": "Session {id} could not be paused (already paused, or nothing to stop).",
};

export const ptBR: Widen<typeof pt> = {
  "governor.bad_config": "Configuração do governador de recursos inválida: {detail}",
  "governor.admission_denied":
    "Não iniciado: a memória está {level} ({free_mb} MB livres). Os agentes esperam ela se recuperar; os workers estão limitados a {max}.",
  "governor.pause_failed": "A sessão {id} não pôde ser pausada (já está pausada ou não há nada para parar).",
};
