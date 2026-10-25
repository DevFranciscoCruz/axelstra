import type { Widen } from "../i18n";

/** Texts of the message codes raised by the removal of a project (`project.remove`; see crates/daemon/src/server/removal.rs). */
export const pt = {
  "project.remove_bad_mode": "O modo de remoção tem de ser «list» ou «all».",
} as const satisfies Record<string, string>;

export const en: Widen<typeof pt> = {
  "project.remove_bad_mode": 'The removal mode must be "list" or "all".',
};

export const ptBR: Widen<typeof pt> = {
  "project.remove_bad_mode": 'O modo de remoção precisa ser "list" ou "all".',
};
