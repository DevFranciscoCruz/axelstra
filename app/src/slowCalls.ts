import { createSignal } from "solid-js";

/**
 * Slowness is information, never a failure: a daemon call that is still waiting after this long is listed here so the
 * app can say "loading... the system is slow (<method>)" with a Cancel button. The call itself goes on waiting for its
 * event (reply, connection closed); only the visual notice and the app-side request are ever dropped.
 */
export const SLOW_NOTICE_MS = 6_000;

type Entry = { id: number; method: string; cancel: () => void };
const [entries, setEntries] = createSignal<Entry[]>([]);
let seq = 0;

/** the slow methods on screen, one row per method (the same read repeated by several panels is one notice) */
export const slowMethods = (): string[] => [...new Set(entries().map((e) => e.method))];

/** Discards the app-side request of every slow call of `method`: each rejects as cancelled, the notice goes away. */
export const cancelSlow = (method: string) => {
  for (const e of entries().filter((x) => x.method === method)) e.cancel();
};

/**
 * Starts watching one pending call; the returned function stops it (the call settled). The notice appears only if the
 * call is still pending after `after` ms.
 */
export const watchSlow = (method: string, cancel: () => void, after = SLOW_NOTICE_MS): (() => void) => {
  const id = ++seq;
  const t = setTimeout(() => setEntries((l) => [...l, { id, method, cancel }]), after);
  return () => {
    clearTimeout(t);
    setEntries((l) => (l.some((e) => e.id === id) ? l.filter((e) => e.id !== id) : l));
  };
};
