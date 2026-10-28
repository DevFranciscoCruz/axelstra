/** the terminal/editor area never gets narrower than this because of the side panels */
export const MIN_CENTER = 360;
/** a side panel is never squeezed below this, whatever the window */
const FLOOR = 120;

export const LEFT = { min: 180, max: 480 } as const;
/** under this window width the right panel is hidden and the project list may take at most `NARROW_LEFT_SHARE` of the window */
export const NARROW_BELOW = 1000;
export const NARROW_LEFT_SHARE = 0.34;
/** The one width of the project list: what `fitPanel` gave, capped to a share of a narrow window. The grid column, the
 *  resize handle and `aria-valuenow` all use this value, so they never disagree. */
export const capLeft = (w: number, win: number): number =>
  win <= NARROW_BELOW ? Math.min(w, Math.round(win * NARROW_LEFT_SHARE)) : w;
export const RIGHT = { min: 300, max: 640 } as const;

/** Width for a side panel: inside its own range, and small enough that the centre keeps MIN_CENTER next to the
 *  other panel (`otherPx`, 0 when that one is hidden) in a window `win` px wide. A very narrow window wins over the range. */
export const fitPanel = (w: number, range: { min: number; max: number }, otherPx: number, win: number): number => {
  const upper = Math.max(FLOOR, Math.min(range.max, win - otherPx - MIN_CENTER));
  const lower = Math.min(range.min, upper);
  return Math.round(Math.min(Math.max(w, lower), upper));
};

/** Keyboard step for a resizer: arrows move 16px (64px with Shift), Home/End jump to the ends; null for other keys.
 *  `dir` is 1 for the left panel (Right grows it) and -1 for the right one (Left grows it). */
export const resizeTo = (
  w: number,
  key: string,
  shift: boolean,
  dir: 1 | -1,
  range: { min: number; max: number },
): number | null => {
  const step = shift ? 64 : 16;
  if (key === "ArrowRight") return Math.min(range.max, Math.max(range.min, w + dir * step));
  if (key === "ArrowLeft") return Math.min(range.max, Math.max(range.min, w - dir * step));
  if (key === "Home") return range.min;
  if (key === "End") return range.max;
  return null;
};

/** a terminal of a split is never narrower than this (about 40 columns) */
export const MIN_SPLIT_PX = 320;

/** Range of the right pane's share for an area `width` px wide: each pane keeps `MIN_SPLIT_PX`, never outside 0.2 to 0.8.
 *  An area too narrow for two panes gets exactly 0.5 (the split still opens, the two halves are just small). */
export const splitBounds = (width: number): { min: number; max: number } => {
  if (!(width > 0)) return { min: 0.2, max: 0.8 };
  const min = Math.min(0.5, Math.max(0.2, MIN_SPLIT_PX / width));
  return { min, max: 1 - min };
};

/** Share limited to the bounds, in whole percents. */
export const clampSplit = (frac: number, width = 0): number => {
  const b = splitBounds(width);
  return Math.round(Math.min(b.max, Math.max(b.min, frac)) * 100) / 100;
};

/** Keyboard step for the split between two terminals; `frac` is the share of the right pane (0.2 to 0.8 by default,
 *  narrower when `width`, the area in px, is given). */
export const splitTo = (frac: number, key: string, width = 0): number | null => {
  const b = splitBounds(width);
  if (key === "ArrowLeft") return clampSplit(frac + 0.05, width);
  if (key === "ArrowRight") return clampSplit(frac - 0.05, width);
  if (key === "Home") return clampSplit(b.max, width);
  if (key === "End") return clampSplit(b.min, width);
  return null;
};

/** Where `moveTab` should put tab `id` to move it one place (`dir` -1 left, 1 right) among `ids`, in strip order: the id
 *  it goes before, `null` for the very end, `undefined` when it already sits at that edge (or is not there). */
export const tabMoveTarget = (ids: number[], id: number, dir: -1 | 1): number | null | undefined => {
  const at = ids.indexOf(id);
  if (at < 0) return undefined;
  if (dir === -1) return at === 0 ? undefined : ids[at - 1];
  if (at === ids.length - 1) return undefined;
  return ids[at + 2] ?? null;
};
