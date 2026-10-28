import type { Widen } from "../i18n";

/** Texts of the message codes raised by the antivirus and indexer exclusions (prefix `scanx.`; see crates/daemon/src/scanx.rs). */
export const pt = {
  "scanx.unsupported": "As exclusões do Defender só existem no Windows.",
  "scanx.no_folders": "Não há nenhuma pasta .axelstra-wt para alterar.",
  "scanx.too_many_paths": "Há pastas a mais para um único comando de administrador ({count}).",
  "scanx.bad_path": "Recusado: só se excluem pastas .axelstra-wt ({path}).",
  "scanx.powershell_failed": "Não foi possível executar o PowerShell: {detail}",
  "scanx.elevation_cancelled": "O pedido de administrador foi cancelado. Nada foi alterado.",
  "scanx.elevation_failed": "Não foi possível iniciar o PowerShell como administrador: {detail}",
  "scanx.defender_blocked":
    "O Defender recusou a alteração (Proteção contra Adulteração ou política da organização): {detail}",
  "scanx.defender_failed": "O Defender não respondeu: {detail}",
  "scanx.busy": "Outra alteração do Defender está à espera do pedido de administrador.",
} as const satisfies Record<string, string>;

export const en: Widen<typeof pt> = {
  "scanx.unsupported": "Defender exclusions only exist on Windows.",
  "scanx.no_folders": "There is no .axelstra-wt folder to change.",
  "scanx.too_many_paths": "Too many folders for a single administrator command ({count}).",
  "scanx.bad_path": "Refused: only .axelstra-wt folders can be excluded ({path}).",
  "scanx.powershell_failed": "Could not run PowerShell: {detail}",
  "scanx.elevation_cancelled": "The administrator prompt was cancelled. Nothing was changed.",
  "scanx.elevation_failed": "Could not start PowerShell as administrator: {detail}",
  "scanx.defender_blocked": "Defender refused the change (Tamper Protection or an organization policy): {detail}",
  "scanx.defender_failed": "Defender did not answer: {detail}",
  "scanx.busy": "Another Defender change is waiting for the administrator prompt.",
};

export const ptBR: Widen<typeof pt> = {
  "scanx.unsupported": "As exclusões do Defender só existem no Windows.",
  "scanx.no_folders": "Não há nenhuma pasta .axelstra-wt para alterar.",
  "scanx.too_many_paths": "Há pastas demais para um único comando de administrador ({count}).",
  "scanx.bad_path": "Recusado: só é possível excluir pastas .axelstra-wt ({path}).",
  "scanx.powershell_failed": "Não foi possível executar o PowerShell: {detail}",
  "scanx.elevation_cancelled": "O pedido de administrador foi cancelado. Nada foi alterado.",
  "scanx.elevation_failed": "Não foi possível iniciar o PowerShell como administrador: {detail}",
  "scanx.defender_blocked":
    "O Defender recusou a alteração (Proteção contra Violação ou política da organização): {detail}",
  "scanx.defender_failed": "O Defender não respondeu: {detail}",
  "scanx.busy": "Outra alteração do Defender está esperando o pedido de administrador.",
};
