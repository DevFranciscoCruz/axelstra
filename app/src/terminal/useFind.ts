import type { Terminal } from "@xterm/xterm";
import type { SearchAddon } from "@xterm/addon-search";
import { createSignal } from "solid-js";

/** The find bar's state. Created with the component (the JSX needs it), bound to the terminal once that exists. */
export function useFind() {
  const [finding, setFinding] = createSignal(false);
  let input: HTMLInputElement | undefined;
  let term: Terminal | undefined;
  let search: SearchAddon | undefined;

  const open = () => {
    setFinding(true);
    const sel = term?.getSelection() ?? "";
    queueMicrotask(() => {
      if (sel && input) input.value = sel.split("\n")[0];
      input?.focus();
      input?.select();
    });
  };
  const close = () => {
    setFinding(false);
    search?.clearDecorations();
    term?.focus();
  };
  const run = (back: boolean, incremental = false) => {
    const q = input?.value ?? "";
    if (!q) return void search?.clearDecorations();
    if (back) search?.findPrevious(q);
    else search?.findNext(q, { incremental });
  };
  return {
    finding,
    open,
    close,
    run,
    setInput: (el: HTMLInputElement) => (input = el),
    bind: (t: Terminal, s: SearchAddon) => {
      term = t;
      search = s;
    },
  };
}
