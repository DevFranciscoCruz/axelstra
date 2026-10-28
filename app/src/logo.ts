/** A project logo is cosmetic: whatever goes wrong, the answer is "no logo" (the letter icon), never an error. */
export type LogoAnswer = { data_url?: string | null; loading?: boolean } | null | undefined;

/** After this many failed or still-loading looks in one session a project stays on its fallback. */
export const LOGO_TRIES = 2;

/**
 * `load(id)` -> the data URL, or null when there is none. Never throws or rejects. A found logo and a plain "no logo" are
 * remembered for the session; a timeout, an error or `{loading: true}` (the daemon is still looking) is only a miss, so the
 * next ask (a remount, not a timer) may try once more, and after `LOGO_TRIES` looks the fallback is kept.
 */
export function createLogoLoader(send: (id: string) => Promise<LogoAnswer>) {
  const done = new Map<string, string | null>();
  const misses = new Map<string, number>();
  const inflight = new Map<string, Promise<string | null>>();
  const look = async (id: string): Promise<string | null> => {
    try {
      const r = await send(id);
      if (r && r.loading !== true) {
        const url = typeof r.data_url === "string" && r.data_url ? r.data_url : null;
        done.set(id, url);
        return url;
      }
    } catch (e) {
      console.debug("project.logo", e);
    }
    const n = (misses.get(id) ?? 0) + 1;
    misses.set(id, n);
    if (n >= LOGO_TRIES) done.set(id, null);
    return null;
  };
  return (id: string): Promise<string | null> => {
    if (done.has(id)) return Promise.resolve(done.get(id) ?? null);
    let p = inflight.get(id);
    if (!p) {
      p = look(id).finally(() => inflight.delete(id));
      inflight.set(id, p);
    }
    return p;
  };
}
