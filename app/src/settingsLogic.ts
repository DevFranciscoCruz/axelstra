import type { Diagnostics } from "./api";
import { S } from "./strings";

/** What went wrong loading the settings: a daemon from before the settings existed answers "unknown method"; anything else is a real error. */
export const classifySettingsError = (e: unknown): "old-daemon" | "other" => (/unknown method/i.test(String((e as { message?: unknown })?.message ?? e)) ? "old-daemon" : "other");

/** The error text without the "Error: " prefix the runtime adds to thrown values. */
export const errorText = (e: unknown) => String((e as { message?: unknown })?.message ?? e).replace(/^Error:\s*/, "");

/** Calls `probe` until it succeeds (true) or the tries run out (false). Used to wait for the daemon to come back: each failed
 *  call already took the time the host needs to start and connect to it, so there is nothing to sleep between tries. */
export const waitUntil = async (probe: () => Promise<unknown>, tries: number) => {
  for (let i = 0; i < tries; i++) {
    try {
      await probe();
      return true;
    } catch {
      /* the daemon is not back yet: the next call asks the host to start and reach it again */
    }
  }
  return false;
};

/** Rows for the Diagnostics section from what the backend reported. */
export const diagnosticRows = (d: Diagnostics | null | undefined): { label: string; value: string }[] => {
  if (!d) return [];
  const lag = d.lag_log_bytes ? `${d.lag_log} (${d.lag_log_bytes < 1024 ? `${d.lag_log_bytes} B` : `${Math.round(d.lag_log_bytes / 1024)} KB`})` : d.lag_log;
  return [
    { label: S.diagApp, value: `${d.app_version} · ${S.diagProto} ${d.app_proto}` },
    {
      label: S.diagDaemon,
      value: d.daemon_connected ? `${S.diagProto} ${d.daemon_proto ?? "?"}${d.daemon_instance != null ? ` · ${S.diagInstance} ${d.daemon_instance}` : ""}` : S.diagOffline,
    },
    { label: S.diagData, value: d.data_dir },
    { label: S.diagLag, value: lag },
  ];
};

export const MAX_BUILD_JOBS = 256;
export const MAX_BUILD_MB = 4_194_304;
/** Defaults offered when the user switches a build number from automatic to manual. */
export const DEFAULT_BUILD_JOBS = 4;
export const DEFAULT_BUILD_MB = 8192;
export const DEFAULT_BUILD_MARGIN_MB = 2048;

/** A whole number of 1..max typed in a build field (jobs, lock memory), or null when it is empty or not valid. */
export const parseBuildNumber = (text: string, max: number): number | null => {
  const t = text.trim();
  if (!/^\d{1,9}$/.test(t)) return null;
  const n = Number(t);
  return n >= 1 && n <= max ? n : null;
};
