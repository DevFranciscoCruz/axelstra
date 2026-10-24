import type { Terminal } from "@xterm/xterm";
import { WebglAddon } from "@xterm/addon-webgl";

/**
 * The GPU renderer costs memory per terminal and the browser allows only a few contexts: only the visible
 * terminal has one. `onLost` runs after a lost context fell back to the other renderer (fit the grid again, repaint).
 */
export function useWebgl(term: Terminal, onLost: () => void) {
  let gl: WebglAddon | null = null;
  const load = () => {
    if (gl) return;
    try {
      const g = new WebglAddon();
      g.onContextLoss(() => {
        g.dispose();
        if (gl === g) gl = null;
        onLost();
      });
      term.loadAddon(g);
      gl = g;
    } catch {
      /* canvas renderer fallback */
    }
  };
  const drop = () => {
    gl?.dispose();
    gl = null;
  };
  return { load, drop };
}
