/** Where the browser's own right-click menu may still show: text fields and a real text selection (copy/paste). */
const editable = (t: EventTarget | null) =>
  t instanceof Element && !!t.closest("input, textarea, select, [contenteditable]:not([contenteditable=false])");

export const keepsNativeMenu = (
  e: Pick<MouseEvent, "target">,
  sel: () => string = () => getSelection()?.toString() ?? "",
) => editable(e.target) || sel().trim() !== "";

/**
 * In the packaged app no right-click shows the WebView's menu (Back, Reload, Inspect...). Component handlers run first
 * (bubbling); whatever reaches the document and is not a text field or a selection is swallowed. Dev keeps it for Inspect.
 */
export function installNativeMenuGuard(doc: Document = document, dev: boolean = import.meta.env.DEV) {
  if (dev) return () => {};
  const h = (e: MouseEvent) => {
    if (!e.defaultPrevented && !keepsNativeMenu(e)) e.preventDefault();
  };
  doc.addEventListener("contextmenu", h);
  return () => doc.removeEventListener("contextmenu", h);
}
