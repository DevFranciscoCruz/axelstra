import { createSignal } from "solid-js";

/** Open modals, oldest first. Only the last one (the top) reacts to Esc and Tab. */
const stack: symbol[] = [];
/** how many modals are open, as a signal: a dialog underneath another one goes inert when this changes */
const [depth, setDepth] = createSignal(0);

/** the app shell (`data-app-shell`) is inert while a modal is open: no focus, clicks or reading order behind it */
export const syncShell = () => {
  if (typeof document === "undefined") return;
  const shell = document.querySelector("[data-app-shell]");
  if (!shell) return;
  if (stack.length) shell.setAttribute("inert", "");
  else shell.removeAttribute("inert");
};

export function pushModal(): symbol {
  const id = Symbol("modal");
  stack.push(id);
  setDepth(stack.length);
  syncShell();
  return id;
}

export function popModal(id: symbol) {
  const i = stack.indexOf(id);
  if (i >= 0) stack.splice(i, 1);
  setDepth(stack.length);
  syncShell();
}

/** reactive: read inside an effect to follow modals opening and closing above this one */
export const isTopModal = (id: symbol) => depth() >= 0 && stack[stack.length - 1] === id;

/** Test hook. */
export const modalDepth = () => stack.length;
