import type { Widen } from "../i18n";

/** Texts of the message codes raised by the memory governor (prefix `governor.`; see crates/daemon/src/governor.rs). */
export const pt = {
  "governor.bad_config": "Configuração do governador de recursos inválida: {detail}",
} as const satisfies Record<string, string>;

export const en: Widen<typeof pt> = {
  "governor.bad_config": "Invalid resource governor settings: {detail}",
};

export const ptBR: Widen<typeof pt> = {
  "governor.bad_config": "Configuração do governador de recursos inválida: {detail}",
};
