/** Keyboard shortcuts the cheat sheet (?) lists: [keys, what it does] */
export const SHORTCUTS: [string, string][] = [
  ["Ctrl+K", "Paleta de comandos"],
  ["Ctrl+,", "Definições"],
  ["?", "Esta lista de atalhos"],
  ["Ctrl+B", "Mostrar/ocultar projetos"],
  ["Ctrl+Shift+B", "Mostrar/ocultar painel direito"],
  ["Ctrl+Tab / Ctrl+Shift+Tab", "Separador seguinte / anterior"],
  ["Ctrl+1 … Ctrl+9", "Ir para o separador n"],
  ["Ctrl+Shift+T", "Reabrir o último separador fechado"],
  ["Ctrl+J", "Ir para o próximo agente à espera de resposta"],
  ["Ctrl+Shift+F", "Pesquisar conversas"],
  ["Ctrl+Shift+M", "Ditado por voz"],
  ["Duplo clique num separador", "Mudar o nome"],
  ["Clique do meio num separador", "Fechar"],
];

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
