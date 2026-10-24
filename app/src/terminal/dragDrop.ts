import { getCurrentWebview } from "@tauri-apps/api/webview";

export type DropPayload = { paths: string[]; position: { x: number; y: number } };
type Listener = (p: DropPayload) => void;

const listeners = new Set<Listener>();
let unlisten: (() => void) | null = null;
let starting = false;

const start = () => {
  if (unlisten || starting) return;
  starting = true;
  void getCurrentWebview()
    .onDragDropEvent((ev) => {
      if (ev.payload.type !== "drop") return;
      for (const l of [...listeners]) l(ev.payload);
    })
    .then((u) => {
      starting = false;
      // everyone left while the listener was being registered
      if (listeners.size === 0) u();
      else unlisten = u;
    })
    .catch(() => {
      starting = false;
    });
};

/**
 * Files dropped from the OS arrive as native events carrying paths. One webview listener serves every terminal
 * (it is registered with the first subscriber and released with the last); each subscriber filters by its own rect.
 */
export function onTermDrop(l: Listener): () => void {
  listeners.add(l);
  start();
  return () => {
    listeners.delete(l);
    if (listeners.size === 0 && unlisten) {
      unlisten();
      unlisten = null;
    }
  };
}

/** true when a point (physical pixels, as the native event reports it) falls inside the element */
export function dropInside(el: HTMLElement, pos: { x: number; y: number }, dpr = devicePixelRatio): boolean {
  const r = el.getBoundingClientRect();
  const x = pos.x / dpr;
  const y = pos.y / dpr;
  return !(x < r.left || x > r.right || y < r.top || y > r.bottom);
}
