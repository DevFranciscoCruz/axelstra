import { cheatSheet } from "./commands";

/** Keyboard shortcuts the cheat sheet (?) lists, flat: [keys, what it does]. Built from the command registry. */
export const SHORTCUTS: [string, string][] = cheatSheet().flatMap((g) => g.rows);

/** true when a key press aimed at `el` is meant for a text field (input, editor), not for the terminal or the page */
export const isTextTarget = (el: EventTarget | null): boolean => {
  if (!(el instanceof HTMLElement)) return false;
  // xterm types through a hidden textarea: that is the terminal, whose keys the app shortcuts are meant for
  if (el.closest(".xterm")) return false;
  return el.isContentEditable || !!el.closest("input, textarea, select, [contenteditable=''], [contenteditable='true'], .cm-editor");
};

/** a dialog or the palette is open: shortcuts that act on the page underneath must leave the keys alone */
export const overlayOpen = (): boolean => !!document.querySelector('[role="dialog"], .pal-back');

/** the Mac keeps its shortcuts on Cmd where Windows and Linux use Ctrl */
export const isMac = (platform: string = typeof navigator === "undefined" ? "" : navigator.platform || navigator.userAgent): boolean => /mac|iphone|ipad/i.test(platform);

/** shortcut text as this platform names it: "Ctrl+Shift+B" reads "Cmd+Shift+B" on a Mac */
export const K = (text: string, mac: boolean = isMac()): string => (mac ? text.replace(/\bCtrl\b/g, "Cmd") : text);
