/** the terminal/editor area never gets narrower than this because of the side panels */
export const MIN_CENTER = 360;
/** a side panel is never squeezed below this, whatever the window */
const FLOOR = 120;

export const LEFT = { min: 180, max: 480 } as const;
export const RIGHT = { min: 240, max: 640 } as const;

/** Width for a side panel: inside its own range, and small enough that the centre keeps MIN_CENTER next to the
 *  other panel (`otherPx`, 0 when that one is hidden) in a window `win` px wide. A very narrow window wins over the range. */
export const fitPanel = (w: number, range: { min: number; max: number }, otherPx: number, win: number): number => {
  const upper = Math.max(FLOOR, Math.min(range.max, win - otherPx - MIN_CENTER));
  const lower = Math.min(range.min, upper);
  return Math.round(Math.min(Math.max(w, lower), upper));
};

/** Keyboard step for a resizer: arrows move 16px (64px with Shift), Home/End jump to the ends; null for other keys.
 *  `dir` is 1 for the left panel (Right grows it) and -1 for the right one (Left grows it). */
export const resizeTo = (w: number, key: string, shift: boolean, dir: 1 | -1, range: { min: number; max: number }): number | null => {
  const step = shift ? 64 : 16;
  if (key === "ArrowRight") return Math.min(range.max, Math.max(range.min, w + dir * step));
  if (key === "ArrowLeft") return Math.min(range.max, Math.max(range.min, w - dir * step));
  if (key === "Home") return range.min;
  if (key === "End") return range.max;
  return null;
};

/** Keyboard step for the split between two terminals; `frac` is the share of the right pane (0.2 to 0.8). */
export const splitTo = (frac: number, key: string): number | null => {
  const clamp = (n: number) => Math.round(Math.min(0.8, Math.max(0.2, n)) * 100) / 100;
  if (key === "ArrowLeft") return clamp(frac + 0.05);
  if (key === "ArrowRight") return clamp(frac - 0.05);
  if (key === "Home") return 0.8;
  if (key === "End") return 0.2;
  return null;
};
