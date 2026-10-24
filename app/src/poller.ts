export type PollerOpts = {
  /** skip a tick while the window is hidden (default true) */
  visibleOnly?: boolean;
  /** skip a tick while the previous call of `fn` has not finished (default true) */
  inflight?: boolean;
};

export type Poller = {
  /** runs `fn` now (subject to `inflight`), outside the timer */
  now: () => void;
  stop: () => void;
};

/** setInterval for polling: starts ticking at once, skips hidden windows and overlapping calls, and cannot throw into the timer. */
export const createPoller = (fn: () => unknown | Promise<unknown>, ms: number, opts: PollerOpts = {}): Poller => {
  const visibleOnly = opts.visibleOnly ?? true;
  const guardInflight = opts.inflight ?? true;
  let busy = false;
  const run = () => {
    if (guardInflight && busy) return;
    busy = true;
    let r: unknown;
    try {
      r = fn();
    } catch {
      busy = false;
      return;
    }
    if (!(r && typeof (r as PromiseLike<unknown>).then === "function")) {
      busy = false;
      return;
    }
    Promise.resolve(r).then(
      () => (busy = false),
      () => (busy = false),
    );
  };
  const timer = setInterval(() => {
    if (visibleOnly && typeof document !== "undefined" && document.hidden) return;
    run();
  }, ms);
  return { now: run, stop: () => clearInterval(timer) };
};
