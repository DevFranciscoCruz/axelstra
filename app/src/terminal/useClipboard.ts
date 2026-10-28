import { invoke } from "@tauri-apps/api/core";
import type { Terminal } from "@xterm/xterm";
import { S } from "../strings";
import { showNotice } from "../store";
import { MenuItem, showMenu, showMenuAt } from "../ContextMenu";

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
      showNotice(S.clipboardFail);
    }
  };
  const copy = () => void invoke("clipboard_write", { text: term.getSelection() }).catch(() => 0);
  /** the entries of the terminal's context menu; `find` opens the search bar */
  const items = (find: () => void): MenuItem[] => [
    {
      label: S.ctxCopy,
      disabled: !term.hasSelection(),
      run: () => {
        copy();
        term.clearSelection();
      },
    },
    { label: S.ctxPaste, run: () => void paste().then(() => term.focus()) },
    { label: S.ctxSelectAll, run: () => term.selectAll() },
    { separator: true },
    { label: S.ctxFind, run: find },
    { label: S.ctxClear, run: () => term.clear() },
  ];
  /** right click (or the menu key): a menu, never an instant paste of possibly many lines */
  const contextMenu = (find: () => void) => (e: MouseEvent) => showMenu(e, items(find));
  const menuAt = (x: number, y: number, find: () => void) => showMenuAt(x, y, items(find));
  return { paste, copy, contextMenu, menuAt };
}
