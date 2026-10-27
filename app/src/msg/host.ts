import type { Widen } from "../i18n";

/** Texts of the message codes raised while the Tauri host starts the daemon (prefix `daemon.`; see app/src-tauri/src/main.rs). */
export const pt = {
  "daemon.start_failed": "O daemon não arrancou. Fecha a app e abre-a outra vez.",
} as const satisfies Record<string, string>;

export const en: Widen<typeof pt> = {
  "daemon.start_failed": "The daemon did not start. Close the app and open it again.",
};

export const ptBR: Widen<typeof pt> = {
  "daemon.start_failed": "O daemon não iniciou. Feche o app e abra novamente.",
};
