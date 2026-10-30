import type { Widen } from "../i18n";

/** Texts of the message codes raised by the automatic cleanup of worktrees (prefix `janitor.`; see crates/daemon/src/janitor.rs). */
export const pt = {
  "janitor.removed_closed": "Worktree {branch} removido (integrado em {into}); sessão {sessions} fechada.",
  "janitor.pending":
    "O worktree {branch} está integrado, mas {holder} (pid {pid}) tem a pasta {path} aberta; sai quando esse processo terminar.",
  "janitor.pending_busy": "O worktree {branch} está integrado, mas a pasta {path} ainda não pôde ser apagada: {detail}",
} as const satisfies Record<string, string>;

export const en: Widen<typeof pt> = {
  "janitor.removed_closed": "Worktree {branch} removed (merged into {into}); session {sessions} closed.",
  "janitor.pending":
    "Worktree {branch} is integrated but {holder} (pid {pid}) holds its folder {path}; it goes when that process exits.",
  "janitor.pending_busy": "Worktree {branch} is integrated but its folder {path} could not be removed yet: {detail}",
};

export const ptBR: Widen<typeof pt> = {
  "janitor.removed_closed": "Worktree {branch} removido (integrado em {into}); sessão {sessions} fechada.",
  "janitor.pending":
    "O worktree {branch} está integrado, mas {holder} (pid {pid}) está com a pasta {path} aberta; ele sai quando esse processo terminar.",
  "janitor.pending_busy":
    "O worktree {branch} está integrado, mas a pasta {path} ainda não pôde ser excluída: {detail}",
};
