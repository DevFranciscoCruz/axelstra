import type { Terminal } from "@xterm/xterm";
import type { SearchAddon } from "@xterm/addon-search";
import { createSignal } from "solid-js";

/** match colours (#RRGGBB, xterm needs that format); the active match stands out from the others. Decorations are also
 *  what makes the search addon report how many matches there are. */
const DECORATIONS = {
  matchBackground: "#4b5563",
  matchOverviewRuler: "#4b5563",
  activeMatchBackground: "#b45309",
  activeMatchColorOverviewRuler: "#b45309",
};

/** The find bar's state. Created with the component (the JSX needs it), bound to the terminal once that exists. */
export function useFind() {
  const [finding, setFinding] = createSignal(false);
  /** matches of the current search: null before any search, `count` 0 = nothing found, `index` -1 = past the display limit */
  const [results, setResults] = createSignal<{ index: number; count: number } | null>(null);
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
    setResults(null);
    search?.clearDecorations();
    term?.focus();
  };
  const run = (back: boolean, incremental = false) => {
    const q = input?.value ?? "";
    if (!q) {
      setResults(null);
      return void search?.clearDecorations();
    }
    if (back) search?.findPrevious(q, { decorations: DECORATIONS });
    else search?.findNext(q, { incremental, decorations: DECORATIONS });
  };
  return {
    finding,
    results,
    open,
    close,
    run,
    setInput: (el: HTMLInputElement) => (input = el),
    bind: (t: Terminal, s: SearchAddon) => {
      term = t;
      search = s;
      s.onDidChangeResults((r) => setResults({ index: r.resultIndex, count: r.resultCount }));
    },
  };
}
