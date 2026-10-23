export const OPEN_DELAY = 250;
export const CLOSE_DELAY = 150;

/** Hover intent: open after a short delay, close shortly after the pointer left; a new event cancels the pending one. */
export function hoverIntent(set: (open: boolean) => void, openDelay = OPEN_DELAY, closeDelay = CLOSE_DELAY) {
  let timer: ReturnType<typeof setTimeout> | undefined;
  const cancel = () => {
    if (timer !== undefined) clearTimeout(timer);
    timer = undefined;
  };
  const later = (open: boolean, ms: number) => {
    cancel();
    timer = setTimeout(() => {
      timer = undefined;
      set(open);
    }, ms);
  };
  return { enter: () => later(true, openDelay), leave: () => later(false, closeDelay), cancel };
}

/** Horizontal shift (px, <= 0) that keeps a box of the given right edge inside the window with a margin. */
export const keepInside = (right: number, windowWidth: number, margin = 8) => Math.min(0, windowWidth - margin - right);
