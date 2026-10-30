import type { Widen } from "../i18n";

/** Texts of the message codes raised by the disk side (prefixes `disk.` and `artifacts.`, plus their settings checks; see
 *  crates/daemon/src/server/disk.rs and crates/daemon/src/artifacts.rs). */
export const pt = {
  "disk.low": "Pouco espaço no disco {volume} ({free_gb} GB livres). Worktrees maiores: {top}",
  "artifacts.busy": "{path} está em uso (uma sessão ou um build): os builds ficam.",
  "artifacts.recent": "{path} foi usado nas últimas {hours} h: os builds ficam.",
  "artifacts.main": "{path} é o checkout principal do projeto: os builds ficam.",
  "artifacts.failed": "Não foi possível apagar {path}: {detail}",
  "settings.bad_disk":
    "O disco aceita enabled (true ou false), min_free_gb e margin_gb (GB inteiros) e min_free_pct (de 0 a 90).",
  "settings.bad_janitor": "A limpeza aceita artifact_quiet_hours (um número inteiro de 1 a 720).",
} as const satisfies Record<string, string>;

export const en: Widen<typeof pt> = {
  "disk.low": "Low disk space on {volume} ({free_gb} GB free). Biggest worktrees: {top}",
  "artifacts.busy": "{path} is in use (a session or a build): its builds are kept.",
  "artifacts.recent": "{path} was used in the last {hours} h: its builds are kept.",
  "artifacts.main": "{path} is the project's main checkout: its builds are kept.",
  "artifacts.failed": "Could not remove {path}: {detail}",
  "settings.bad_disk":
    "disk takes enabled (true or false), min_free_gb and margin_gb (whole GB) and min_free_pct (0 to 90).",
  "settings.bad_janitor": "janitor takes artifact_quiet_hours (a whole number from 1 to 720).",
};

export const ptBR: Widen<typeof pt> = {
  "disk.low": "Pouco espaço no disco {volume} ({free_gb} GB livres). Maiores worktrees: {top}",
  "artifacts.busy": "{path} está em uso (uma sessão ou um build): os builds são mantidos.",
  "artifacts.recent": "{path} foi usado nas últimas {hours} h: os builds são mantidos.",
  "artifacts.main": "{path} é o checkout principal do projeto: os builds são mantidos.",
  "artifacts.failed": "Não foi possível excluir {path}: {detail}",
  "settings.bad_disk":
    "O disco aceita enabled (true ou false), min_free_gb e margin_gb (GB inteiros) e min_free_pct (de 0 a 90).",
  "settings.bad_janitor": "A limpeza aceita artifact_quiet_hours (um número inteiro de 1 a 720).",
};
