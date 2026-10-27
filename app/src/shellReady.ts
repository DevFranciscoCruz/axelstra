/**
 * Waits until a freshly spawned shell has printed its prompt: the daemon announces that the session's state changed
 * (`changes`; it goes quiet after its first output) and the screen has text by then. No polling: `deadlineMs` is the
 * one-shot limit for a shell that never says anything (the caller then types anyway).
 */
export function waitShellReady(
  read: () => Promise<string[]>,
  opts: {
    /** subscribes to "some session changed"; returns the unsubscribe */
    changes: (fn: () => void) => () => void;
    deadlineMs?: number;
    /** runs `fn` once after `ms`; returns the cancel (injectable for tests) */
    after?: (fn: () => void, ms: number) => () => void;
  },
): Promise<boolean> {
  const after =
    opts.after ??
    ((fn, ms) => {
      const t = setTimeout(fn, ms);
      return () => clearTimeout(t);
    });
  return new Promise<boolean>((resolve) => {
    let over = false;
    let reading = false;
    let again = false;
    const finish = (ok: boolean) => {
      if (over) return;
      over = true;
      off();
      cancel();
      resolve(ok);
    };
    const check = async () => {
      if (over) return;
      if (reading) {
        again = true;
        return;
      }
      reading = true;
      let text = "";
      try {
        text = (await read()).join("\n").trim();
      } catch {
        /* the session is not readable yet: the next change looks again */
      }
      reading = false;
      if (text) finish(true);
      else if (again) {
        again = false;
        void check();
      }
    };
    const off = opts.changes(() => void check());
    const cancel = after(() => finish(false), opts.deadlineMs ?? 6000);
  });
}
