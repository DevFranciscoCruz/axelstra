import { onDaemonEvent, type DaemonEvent, type DaemonEventData } from "./api";

export type LiveOpts = {
  /** only events for which this says true run `load` (e.g. the `project` of a `kanban.changed`) */
  match?: (d: DaemonEventData) => boolean;
  /** run `load` when the window comes back (default true): the one-shot refresh that stands in for an event an older daemon never sends */
  onFocus?: boolean;
  /**
   * Event runs wait this long after the FIRST event of a burst and then run once (a trailing debounce anchored on the first
   * event, so a steady stream still gives at most one run per window). `now()` and the window showing again run at once.
   */
  debounceMs?: number;
  /**
   * Events that are too chatty to load on (`session.changed` comes about once a second): they run `load` only when `sig()`
   * differs from what it was at the last run, so a change that does not touch the data costs nothing.
   */
  soft?: { events: DaemonEvent[]; sig: () => string };
};

export type Live = {
  /** runs `load` now (coalesced with a run in flight) */
  now: () => void;
  stop: () => void;
};

/**
 * Keeps something fresh without a timer: `load` runs when the daemon announces one of `events` (and when the stream
 * reconnects, which `onDaemonEvent` already reports), and once when the window shows again. An event that arrives
 * while `load` is still running makes it run once more afterwards, so none is lost and none runs twice at once.
 * Events that arrive while the window is hidden are skipped (the window showing again looks once), unless `onFocus` is off.
 * It does not run `load` for the first time itself: call `now()`.
 */
export const createLive = (
  load: () => unknown | Promise<unknown>,
  events: DaemonEvent[],
  opts: LiveOpts = {},
): Live => {
  let busy = false;
  let again = false;
  let stopped = false;
  let lastSig: string | undefined;
  let timer: ReturnType<typeof setTimeout> | undefined;
  const run = () => {
    if (stopped) return;
    if (busy) {
      again = true;
      return;
    }
    busy = true;
    if (opts.soft) lastSig = opts.soft.sig();
    const settle = () => {
      busy = false;
      if (again) {
        again = false;
        run();
      }
    };
    let r: unknown;
    try {
      r = load();
    } catch {
      settle();
      return;
    }
    if (r && typeof (r as PromiseLike<unknown>).then === "function") Promise.resolve(r).then(settle, settle);
    else settle();
  };
  const watchFocus = (opts.onFocus ?? true) && typeof document !== "undefined";
  const schedule = () => {
    if (stopped || (watchFocus && document.hidden)) return;
    if (!opts.debounceMs) return run();
    if (timer !== undefined) return;
    timer = setTimeout(() => {
      timer = undefined;
      run();
    }, opts.debounceMs);
  };
  const offs = events.map((e) =>
    onDaemonEvent(e, (d) => {
      if (!opts.match || opts.match(d)) schedule();
    }),
  );
  const soft = opts.soft;
  if (soft)
    for (const e of soft.events)
      offs.push(
        onDaemonEvent(e, () => {
          if (soft.sig() !== lastSig) schedule();
        }),
      );
  const onVisible = () => {
    if (!document.hidden) run();
  };
  if (watchFocus) {
    document.addEventListener("visibilitychange", onVisible);
    window.addEventListener("focus", onVisible);
  }
  return {
    now: run,
    stop: () => {
      stopped = true;
      if (timer !== undefined) clearTimeout(timer);
      timer = undefined;
      for (const off of offs) off();
      if (watchFocus) {
        document.removeEventListener("visibilitychange", onVisible);
        window.removeEventListener("focus", onVisible);
      }
    },
  };
};
