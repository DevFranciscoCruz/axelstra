/**
 * Keeps a PTY's size equal to what its xterm shows. The agent draws for the size the PTY has, so a mismatch wraps and
 * overlaps the whole screen. The size is sent only while the terminal is visible, remembered once the daemon accepted
 * it, and sent again (also after a failure) whenever it differs from what the PTY was told.
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
  const push = () => {
    clearTimeout(timer);
    if (d.gone() || !d.visible()) return;
    const { cols, rows } = d.size();
    const key = `${cols}x${rows}`;
    if (key === told) return;
    d.send(cols, rows).then(
      () => {
        const now = d.size();
        if (now.cols === cols && now.rows === rows) told = key;
      },
      () => {
        timer = setTimeout(push, 500);
      },
    );
  };
  return {
    push,
    /** dragging a panel fires many sizes; only the last one is sent (each resize makes the agent redraw everything) */
    resized: () => {
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
