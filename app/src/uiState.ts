import { getSettings, setSettings } from "./api";
import { setMirror } from "./persist";

/**
 * UI choices that used to live only in the WebView's localStorage (pinned worktrees, folded groups, the notification
 * list, ...) are also kept in the daemon's settings.json, so a changed WebView profile or a reinstall does not lose them.
 * localStorage stays the copy the signals read synchronously; `syncUi` runs before the app is imported.
 */

/** Read at import time by modules that load before the sync (theme, terminal font), or device-local: not mirrored. */
const LOCAL_ONLY = new Set(["axelstra.theme", "axelstra.terminal", "axelstra.updateDismissed"]);
export const isMirrored = (key: string) => key.startsWith("axelstra.") && !LOCAL_ONLY.has(key);

type Store = Pick<Storage, "getItem" | "setItem" | "key" | "length">;
export type UiDeps = {
  load: () => Promise<Record<string, string>>;
  save: (patch: Record<string, string | null>) => Promise<unknown>;
  storage: Store;
  /** how long a burst of writes waits before it is sent */
  delayMs?: number;
};

/**
 * The daemon's copy wins for the keys it has (a fresh profile gets them back); keys only this profile has are sent to it
 * (the one-time migration). After that every write is sent in a debounced batch. Never throws: without a daemon answer
 * the app runs on localStorage alone, as before.
 */
export async function syncUi(d: UiDeps): Promise<{ restored: string[]; migrated: string[] } | null> {
  let remote: Record<string, string>;
  try {
    remote = await d.load();
  } catch {
    return null;
  }
  const restored: string[] = [];
  for (const [k, v] of Object.entries(remote)) {
    if (!isMirrored(k) || typeof v !== "string") continue;
    try {
      if (d.storage.getItem(k) !== v) {
        d.storage.setItem(k, v);
        restored.push(k);
      }
    } catch {
      /* storage unavailable: the app runs on the daemon's values only for this session */
    }
  }
  const local: Record<string, string> = {};
  try {
    for (let i = 0; i < d.storage.length; i++) {
      const k = d.storage.key(i);
      const v = k === null ? null : d.storage.getItem(k);
      if (k !== null && v !== null && isMirrored(k) && !(k in remote)) local[k] = v;
    }
  } catch {
    /* nothing to migrate */
  }
  const migrated = Object.keys(local);
  if (migrated.length) await d.save(local).catch(() => undefined);

  let queued: Record<string, string | null> = {};
  let timer: ReturnType<typeof setTimeout> | undefined;
  const flush = () => {
    timer = undefined;
    const batch = queued;
    queued = {};
    void d.save(batch).catch((e) => console.debug("ui state not saved", e));
  };
  setMirror((k, v) => {
    if (!isMirrored(k)) return;
    queued[k] = v;
    timer ??= setTimeout(flush, d.delayMs ?? 400);
  });
  return { restored, migrated };
}

/** `syncUi` against the daemon and localStorage, giving up after `timeoutMs` so a slow daemon cannot hold the window blank. */
export async function hydrateUi(timeoutMs = 2000): Promise<void> {
  let storage: Store;
  try {
    storage = localStorage;
  } catch {
    return;
  }
  const slow = new Promise<null>((r) => setTimeout(() => r(null), timeoutMs));
  await Promise.race([
    syncUi({ load: () => getSettings().then((s) => s.ui ?? {}), save: (patch) => setSettings({ ui: patch }), storage }),
    slow,
  ]);
}
