import type { Diagnostics } from "./api";

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

/** Rows for the Diagnostics section from whatever the backend reported; missing parts are left out. */
export const diagnosticRows = (d: Diagnostics | null | undefined): { label: string; value: string }[] => {
  if (!d) return [];
  const rows: { label: string; value: string }[] = [];
  if (d.daemon?.version !== undefined || d.daemon?.proto !== undefined) {
    rows.push({ label: "daemon", value: [d.daemon?.version, d.daemon?.proto !== undefined ? `proto ${d.daemon.proto}` : ""].filter(Boolean).join(" · ") });
  }
  for (const [k, v] of Object.entries(d.paths ?? {})) rows.push({ label: k, value: String(v) });
  if (d.lag_log) rows.push({ label: "lag.log", value: d.lag_log });
  if (d.lag !== undefined && d.lag !== null) rows.push({ label: "lag", value: typeof d.lag === "string" ? d.lag : JSON.stringify(d.lag) });
  return rows;
};
