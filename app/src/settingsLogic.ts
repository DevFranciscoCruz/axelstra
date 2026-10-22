import type { Diagnostics } from "./api";
import { S } from "./strings";

/** What went wrong loading the settings: a daemon from before the settings existed answers "unknown method"; anything else is a real error. */
export const classifySettingsError = (e: unknown): "old-daemon" | "other" => (/unknown method/i.test(String((e as { message?: unknown })?.message ?? e)) ? "old-daemon" : "other");

/** The error text without the "Error: " prefix the runtime adds to thrown values. */
export const errorText = (e: unknown) => String((e as { message?: unknown })?.message ?? e).replace(/^Error:\s*/, "");

/** Keeps calling `probe` until it succeeds (true) or the tries run out (false). Used to wait for the daemon to come back. */
export const waitUntil = async (probe: () => Promise<unknown>, tries: number, delay: number, sleep = (ms: number) => new Promise<void>((r) => setTimeout(r, ms))) => {
  for (let i = 0; i < tries; i++) {
    try {
      await probe();
      return true;
    } catch {
      await sleep(delay);
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
