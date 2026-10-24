import { invoke } from "@tauri-apps/api/core";
import type { Terminal } from "@xterm/xterm";

/** a path with spaces is typed quoted */
export const quotePath = (p: string) => (/\s/.test(p) ? `"${p}"` : p);

/**
 * WebView2 gates navigator.clipboard behind a permission prompt, so the clipboard goes through the native
 * side. Images are saved to a temp PNG and typed as a path.
 */
export function useClipboard(term: Terminal) {
  const paste = async () => {
    try {
      const c = await invoke<{ kind: string; value: string }>("clipboard_read");
      if (c.kind === "image") term.paste(quotePath(c.value) + " ");
      else if (c.kind === "text") term.paste(c.value);
    } catch {
      /* clipboard unavailable */
    }
  };
  const copy = () => void invoke("clipboard_write", { text: term.getSelection() }).catch(() => 0);
  /** right click: copy a selection, otherwise paste (Windows Terminal behaviour) */
  const contextMenu = (e: MouseEvent) => {
    e.preventDefault();
    if (term.hasSelection()) {
      copy();
      term.clearSelection();
    } else void paste();
  };
  return { paste, copy, contextMenu };
}
