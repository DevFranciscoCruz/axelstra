import type { GovernorConfig, MemLevel, MemStatus } from "./api";

/** One frozen session or build, as `mem.status` lists it (`session.paused` adds one, `session.resumed` takes it away). */
export type PausedItem = MemStatus["paused"][number];
export type TopSession = { session: number; rss_mb: number };

export type MemState = {
  /** false until the first `mem.status` or `memory.pressure` answered */
  loaded: boolean;
  level: MemLevel;
  free_mb: number;
  total_mb: number;
  top: TopSession[];
  paused: PausedItem[];
  config: GovernorConfig | null;
};

export const initialMem = (): MemState => ({
  loaded: false,
  level: "normal",
  free_mb: 0,
  total_mb: 0,
  top: [],
  paused: [],
  config: null,
});

const LEVELS: readonly MemLevel[] = ["normal", "high", "critical"];
const asLevel = (v: unknown): MemLevel | null => (LEVELS.includes(v as MemLevel) ? (v as MemLevel) : null);
const num = (v: unknown): number | null => (typeof v === "number" && Number.isFinite(v) ? v : null);

/** Biggest first; rows that do not carry a session id and a size are dropped. */
const cleanTop = (v: unknown): TopSession[] =>
  (Array.isArray(v) ? v : [])
    .flatMap((r) => {
      const session = num(r?.session ?? r?.id);
      const rss = num(r?.rss_mb);
      return session === null || rss === null ? [] : [{ session, rss_mb: rss }];
    })
    .sort((a, b) => b.rss_mb - a.rss_mb);

/** `memory.pressure {level, free_mb, total_mb, top}`: the level moved or the free memory changed noticeably. */
export const applyPressure = (s: MemState, ev: Record<string, unknown>): MemState => {
  const level = asLevel(ev.level);
  if (!level) return s;
  return {
    ...s,
    loaded: true,
    level,
    free_mb: num(ev.free_mb) ?? s.free_mb,
    total_mb: num(ev.total_mb) ?? s.total_mb,
    top: ev.top === undefined ? s.top : cleanTop(ev.top),
  };
};

/** `mem.status`: the whole picture, on bootstrap and after a reconnect. */
export const applyStatus = (s: MemState, st: MemStatus): MemState => ({
  loaded: true,
  level: asLevel(st.level) ?? s.level,
  free_mb: st.free_mb,
  total_mb: st.total_mb,
  top: cleanTop(st.sessions),
  paused: st.paused ?? [],
  config: st.config ?? s.config,
});

/** `session.paused {id, build, reason}`; a session that was already listed only has its reason refreshed. */
export const applyPaused = (s: MemState, ev: Record<string, unknown>, now: number): MemState => {
  const id = num(ev.id);
  if (id === null) return s;
  const build = typeof ev.build === "string" && ev.build ? ev.build : null;
  const item: PausedItem = {
    kind: build ? "build" : "session",
    session: id,
    reason: typeof ev.reason === "string" ? ev.reason : "memory",
    since: now,
    name: build,
  };
  const rest = s.paused.filter((p) => !(p.session === id && p.kind === item.kind));
  return { ...s, paused: [...rest, item] };
};

/** `session.resumed {id}`: whatever was frozen under that session is running again. */
export const applyResumed = (s: MemState, ev: Record<string, unknown>): MemState => {
  const id = num(ev.id);
  if (id === null || !s.paused.some((p) => p.session === id)) return s;
  return { ...s, paused: s.paused.filter((p) => p.session !== id) };
};

/** The level change worth telling the user about, or null: nothing before the first reading, nothing when it stayed. */
export const levelTransition = (prev: MemState, next: MemState): MemLevel | null =>
  prev.loaded && next.loaded && prev.level !== next.level ? next.level : null;

/** Items that appeared in / left the paused list between two states. */
export const pausedDelta = (prev: MemState, next: MemState): { paused: PausedItem[]; resumed: PausedItem[] } => {
  const key = (p: PausedItem) => `${p.kind}:${p.session}`;
  const had = new Set(prev.paused.map(key));
  const has = new Set(next.paused.map(key));
  return {
    paused: next.paused.filter((p) => !had.has(key(p))),
    resumed: prev.paused.filter((p) => !has.has(key(p))),
  };
};

/** CSS class of a level (status bar chip, panel): "normal" has no colour of its own. */
export const levelClass = (level: MemLevel): string =>
  level === "critical" ? "mem-critical" : level === "high" ? "mem-high" : "mem-normal";

/** The chip is only worth the space when the machine is under pressure, or when a session is frozen. */
export const chipVisible = (s: MemState): boolean => s.loaded && (s.level !== "normal" || s.paused.length > 0);

export type Thresholds = Pick<GovernorConfig, "high_free_pct" | "crit_free_pct" | "high_free_mb" | "crit_free_mb">;
export type ThresholdProblem = "pct_range" | "mb_range" | "pct_order" | "mb_order";

/** Why these thresholds would be refused (critical must be stricter than high; percentages within 1-99; sizes positive), or null. */
export const thresholdProblem = (t: Thresholds): ThresholdProblem | null => {
  const pcts = [t.high_free_pct, t.crit_free_pct];
  const mbs = [t.high_free_mb, t.crit_free_mb];
  if (pcts.some((v) => !Number.isFinite(v) || v < 1 || v > 99)) return "pct_range";
  if (mbs.some((v) => !Number.isFinite(v) || v < 1)) return "mb_range";
  if (t.crit_free_pct >= t.high_free_pct) return "pct_order";
  if (t.crit_free_mb >= t.high_free_mb) return "mb_order";
  return null;
};

/** A typed number (also with a decimal comma), or null when it is not one. */
export const parseAmount = (raw: string): number | null => {
  const t = raw.trim().replace(",", ".");
  if (!/^\d+(\.\d+)?$/.test(t)) return null;
  return Number(t);
};

/** Megabytes split for display: whole MB under a gigabyte, GB with one decimal above. */
export const mbParts = (mb: number): { value: number; digits: number; unit: "MB" | "GB" } =>
  mb >= 1024 ? { value: mb / 1024, digits: 1, unit: "GB" } : { value: Math.round(mb), digits: 0, unit: "MB" };
