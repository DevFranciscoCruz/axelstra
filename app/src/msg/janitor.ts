import type { Widen } from "../i18n";

/** Texts of the message codes raised by the automatic cleanup of worktrees (prefix `janitor.`; see crates/daemon/src/janitor.rs). */
export const pt = {
  "janitor.removed_closed": "Worktree {branch} removido (integrado em {into}); sessão {sessions} fechada.",
} as const satisfies Record<string, string>;

export const en: Widen<typeof pt> = {
  "janitor.removed_closed": "Worktree {branch} removed (merged into {into}); session {sessions} closed.",
};

export const ptBR: Widen<typeof pt> = {
  "janitor.removed_closed": "Worktree {branch} removido (integrado em {into}); sessão {sessions} fechada.",
};
