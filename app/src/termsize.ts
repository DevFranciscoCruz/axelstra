/**
 * Keeps a PTY's size equal to what its xterm shows. The agent draws for the size the PTY has, so a mismatch wraps and
 * overlaps the whole screen. The size is sent only while the terminal is visible, remembered once the daemon accepted
 * it, and sent again (also after a failure) whenever it differs from what the PTY was told.
 *
 * xterm reflows its wrapped lines on every column change. When the grid went away and came back to the size the PTY
 * already has (a DPI or scrollbar flicker: 120 -> 119 -> 120 within the debounce), the sizes are equal but the screen
 * was reflowed twice and the agent never redrew, so its relative cursor moves land on the wrong row. That case is
 * sent as a nudge (one row less, then the real size): the size really changes twice, the PTY signals the agent, and
 * it redraws for the grid xterm now has.
 */
export type SizeDeps = {
  size: () => { cols: number; rows: number };
  visible: () => boolean;
  gone: () => boolean;
  send: (cols: number, rows: number) => Promise<unknown>;
};

export const createSizeSync = (d: SizeDeps) => {
  let told = "";
  let timer: ReturnType<typeof setTimeout> | undefined;
  // true once xterm resized (so it reflowed) after the last size the PTY was told
  let reflowed = false;
  // after a failed send the size is tried once more; a second failure waits for the next resize or show (no retry loop)
  let retried = false;
  const push = () => {
    clearTimeout(timer);
    if (d.gone() || !d.visible()) return;
    const { cols, rows } = d.size();
    const key = `${cols}x${rows}`;
    const nudge = key === told && reflowed;
    if (key === told && !nudge) return;
    reflowed = false;
    // a normal resize calls send right away; the nudge chains the real size after the shorter one
    const sent = nudge ? d.send(cols, Math.max(1, rows - 1)).then(() => d.send(cols, rows)) : d.send(cols, rows);
    sent.then(
      () => {
        retried = false;
        const now = d.size();
        if (now.cols === cols && now.rows === rows) told = key;
      },
      () => {
        reflowed = reflowed || nudge;
        if (retried) return;
        retried = true;
        timer = setTimeout(push, 500);
      },
    );
  };
  return {
    push,
    /** dragging a panel fires many sizes; only the last one is sent (each resize makes the agent redraw everything) */
    resized: () => {
      reflowed = true;
      retried = false;
      clearTimeout(timer);
      timer = setTimeout(push, 80);
    },
    /** a fresh subscription: tell the PTY the size again */
    forget: () => {
      told = "";
    },
    cancel: () => clearTimeout(timer),
  };
};

/**
 * Collects the output chunks that arrive between two frames and writes them to xterm in one call: a burst of small
 * chunks otherwise costs one parse/render pass each. `delay` says how long to wait (a frame while visible, longer
 * while the terminal is hidden); a big backlog is written at once so memory does not pile up.
 */
export const createWriteBatcher = (
  write: (data: Uint8Array) => void,
  opts: { delay: () => number; maxBytes?: number } = { delay: () => 8 },
) => {
  const max = opts.maxBytes ?? 256 * 1024;
  let chunks: Uint8Array[] = [];
  let bytes = 0;
  let timer: ReturnType<typeof setTimeout> | undefined;
  const flush = () => {
    clearTimeout(timer);
    timer = undefined;
    if (!chunks.length) return;
    let out: Uint8Array;
    if (chunks.length === 1) out = chunks[0];
    else {
      out = new Uint8Array(bytes);
      let at = 0;
      for (const c of chunks) {
        out.set(c, at);
        at += c.length;
      }
    }
    chunks = [];
    bytes = 0;
    write(out);
  };
  return {
    push: (c: Uint8Array) => {
      chunks.push(c);
      bytes += c.length;
      if (bytes >= max) flush();
      else timer ??= setTimeout(flush, opts.delay());
    },
    flush,
    cancel: () => {
      clearTimeout(timer);
      timer = undefined;
      chunks = [];
      bytes = 0;
    },
  };
};

/** The daemon has no such session (the host passes its English text on, or the message code): it ended and left the registry. */
export const isNoSuchSession = (e: unknown): boolean => {
  if (e && typeof e === "object" && (e as { code?: unknown }).code === "session.not_found") return true;
  const text =
    typeof e === "string" ? e : e instanceof Error ? e.message : ((e as { message?: unknown } | null)?.message ?? "");
  return typeof text === "string" && /no such session/i.test(text);
};

/**
 * Runs `attempt` and, when it fails, once more after `wait` ms. The returned promise covers both tries, so whoever
 * awaits it knows the final outcome: `true` attached, `false` gave up (`onFail` got the last error).
 */
export const attachWithRetry = async <T>(
  attempt: () => Promise<T>,
  wait: number,
  opts: {
    sleep?: (ms: number) => Promise<void>;
    gone?: () => boolean;
    onFail?: (e: unknown) => void;
    /** the failure says the session no longer exists: no retry and no `onFail`, `onEnded` runs instead */
    onEnded?: () => void;
  } = {},
): Promise<{ ok: true; value: T } | { ok: false }> => {
  const sleep = opts.sleep ?? ((ms) => new Promise<void>((r) => setTimeout(r, ms)));
  try {
    return { ok: true, value: await attempt() };
  } catch (first) {
    if (opts.onEnded && isNoSuchSession(first)) {
      opts.onEnded();
      return { ok: false };
    }
    await sleep(wait);
    if (opts.gone?.()) return { ok: false };
    try {
      return { ok: true, value: await attempt() };
    } catch (e) {
      if (opts.onEnded && isNoSuchSession(e)) opts.onEnded();
      else opts.onFail?.(e);
      return { ok: false };
    }
  }
};
