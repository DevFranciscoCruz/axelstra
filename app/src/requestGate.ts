/**
 * Request volume control for daemon reads that git or the filesystem answer slowly. No timers poll anything: a gate only
 * delays or merges calls that something else (a daemon event, a click, the window showing again) already asked for.
 */

/**
 * Wraps `load` so that calls made while one is running share it, and calls made during the run (events that arrive in a
 * burst) cause exactly one more run afterwards. `minGapMs` > 0 also holds a new run back until that long after the last
 * one started, with a single trailing timer (a debounce, not a poll: it exists only when a call is pending).
 * The returned promise settles when the run that covers the call has finished.
 */
export const coalesced = <T>(load: () => Promise<T>, minGapMs = 0): (() => Promise<T | undefined>) => {
  let running: Promise<T | undefined> | null = null;
  let queued: Promise<T | undefined> | null = null;
  let lastStart = 0;

  const start = (): Promise<T | undefined> => {
    lastStart = Date.now();
    const p: Promise<T | undefined> = load().finally(() => {
      if (running === p) running = null;
    });
    running = p;
    return p;
  };

  const later = (after: Promise<unknown>): Promise<T | undefined> => {
    queued = new Promise<T | undefined>((resolve, reject) => {
      const go = () => {
        queued = null;
        const wait = lastStart + minGapMs - Date.now();
        if (wait > 0) {
          setTimeout(() => start().then(resolve, reject), wait);
        } else {
          start().then(resolve, reject);
        }
      };
      after.then(go, go);
    });
    return queued;
  };

  return () => {
    if (queued) return queued;
    if (running) return later(running);
    const wait = lastStart + minGapMs - Date.now();
    if (minGapMs > 0 && wait > 0) return later(Promise.resolve());
    return start();
  };
};

/**
 * Runs `fn` once, `ms` after the FIRST call of a burst (later calls inside the window are covered by that run), so a steady
 * stream of calls gives at most one run per window. The timer exists only while a call is pending; `skip()` true drops the call.
 */
export const debounced = (fn: () => unknown, ms: number, skip: () => boolean = () => false): (() => void) => {
  let timer: ReturnType<typeof setTimeout> | undefined;
  return () => {
    if (timer !== undefined || skip()) return;
    timer = setTimeout(() => {
      timer = undefined;
      void fn();
    }, ms);
  };
};

/** In-flight dedupe by key: the same call made while one is running gets the same promise. */
const inflight = new Map<string, Promise<unknown>>();
export const singleFlight = <T>(key: string, fn: () => Promise<T>): Promise<T> => {
  const hit = inflight.get(key);
  if (hit) return hit as Promise<T>;
  const p = fn().finally(() => {
    if (inflight.get(key) === p) inflight.delete(key);
  });
  inflight.set(key, p);
  return p;
};

/** For tests: forget everything in flight. */
export const resetGates = () => inflight.clear();

/**
 * "Stale" bookkeeping for panels that are not visible: a refresh asked while hidden only marks it, and becoming visible
 * runs it once, and only if something asked in between (or it never ran).
 */
export const lazyRefresh = (load: () => unknown, visible: () => boolean) => {
  let stale = true;
  return {
    /** an event says the data moved */
    ask: () => {
      stale = true;
      if (visible()) {
        stale = false;
        void load();
      }
    },
    /** the panel or the window became visible */
    shown: () => {
      if (stale && visible()) {
        stale = false;
        void load();
      }
    },
  };
};
