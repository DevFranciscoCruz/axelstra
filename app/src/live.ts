import { onDaemonEvent, type DaemonEvent, type DaemonEventData } from "./api";

export type LiveOpts = {
  /** only events for which this says true run `load` (e.g. the `project` of a `kanban.changed`) */
  match?: (d: DaemonEventData) => boolean;
  /** run `load` when the window comes back (default true): the one-shot refresh that stands in for an event an older daemon never sends */
  onFocus?: boolean;
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
  const run = () => {
    if (stopped) return;
    if (busy) {
      again = true;
      return;
    }
    busy = true;
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
  const offs = events.map((e) =>
    onDaemonEvent(e, (d) => {
      if (!opts.match || opts.match(d)) run();
    }),
  );
  const onVisible = () => {
    if (!document.hidden) run();
  };
  const watchFocus = (opts.onFocus ?? true) && typeof document !== "undefined";
  if (watchFocus) {
    document.addEventListener("visibilitychange", onVisible);
    window.addEventListener("focus", onVisible);
  }
  return {
    now: run,
    stop: () => {
      stopped = true;
      for (const off of offs) off();
      if (watchFocus) {
        document.removeEventListener("visibilitychange", onVisible);
        window.removeEventListener("focus", onVisible);
      }
    },
  };
};
