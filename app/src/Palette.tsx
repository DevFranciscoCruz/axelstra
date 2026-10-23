import { For, Show, createEffect, createMemo, createSignal, onCleanup, onMount } from "solid-js";
import { moveIndex } from "./a11y";
import { Modal } from "./Modal";
import { findWorktree, projects, selectWorktree, selectedWorktree, setRightTab, setShowKanban } from "./store";
import { Input } from "./ui";

export type PaletteAction = { label: string; hint?: string; run: () => void };

type Item = { label: string; hint: string; run: () => void };

/** more than this many matches is noise; typing narrows it */
const MAX_SHOWN = 200;

/** Ctrl/Cmd+K: jump to any worktree or run an action from one search box */
export function Palette(props: { actions: () => PaletteAction[] }) {
  const [open, setOpen] = createSignal(false);
  const [q, setQ] = createSignal("");
  const [sel, setSel] = createSignal(0);

  onMount(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        e.stopPropagation();
        setQ("");
        setSel(0);
        setOpen(!open());
      }
    };
    // captured, like Ctrl+B: the terminal must never swallow it
    window.addEventListener("keydown", onKey, true);
    onCleanup(() => window.removeEventListener("keydown", onKey, true));
  });

  const items = createMemo<Item[]>(() => {
    const out: Item[] = props.actions().map((a) => ({ label: a.label, hint: a.hint ?? "ação", run: a.run }));
    // panels and the board belong to a worktree: offered only when one is open (global entries come from `actions`)
    if (findWorktree(selectedWorktree())) {
      out.push(
        { label: "Ficheiros", hint: "painel", run: () => setRightTab("files") },
        { label: "Alterações", hint: "painel", run: () => setRightTab("changes") },
        { label: "PRs e problemas", hint: "painel", run: () => setRightTab("pulls") },
        { label: "Kanban", hint: "vista", run: () => setShowKanban(true) },
      );
    }
    for (const p of projects()) {
      for (const w of p.worktrees) {
        const branch = w.is_main ? (p.default_branch ?? "main") : (w.branch ?? "(destacado)");
        out.push({ label: `${p.name} / ${branch}`, hint: "worktree", run: () => selectWorktree(w.path) });
      }
    }
    return out;
  });

  const shown = createMemo(() => {
    const words = q().toLowerCase().split(/\s+/).filter(Boolean);
    return items().filter((i) => words.every((w) => i.label.toLowerCase().includes(w))).slice(0, MAX_SHOWN);
  });

  createEffect(() => {
    shown();
    setSel(0);
  });
  // keep the highlighted row visible while arrowing through a long list
  createEffect(() => {
    sel();
    queueMicrotask(() => document.querySelector(".pal-item.on")?.scrollIntoView({ block: "nearest" }));
  });

  const choose = (i: Item | undefined) => {
    setOpen(false);
    i?.run();
  };

  // a modal dialog: Esc closes only this one (the dialog underneath stays), Tab stays inside, focus goes back on close
  return (
    <Show when={open()}>
      <Modal label="Paleta de comandos" onClose={() => setOpen(false)} backClass="pal-back" class="pal">
        <Input
          class="pal-input"
          role="combobox"
          aria-expanded="true"
          aria-controls="pal-list"
          aria-activedescendant={shown().length ? `pal-${sel()}` : undefined}
          aria-label="Paleta de comandos"
          placeholder="Ir para worktree ou executar ação…"
          value={q()}
          onInput={(e) => setQ(e.currentTarget.value)}
          onKeyDown={(e) => {
            // Home and End stay with the text field (cursor)
            const to = e.key.startsWith("Arrow") ? moveIndex(sel(), shown().length, e.key) : null;
            if (to !== null) {
              e.preventDefault();
              setSel(to);
            } else if (e.key === "Enter") choose(shown()[sel()]);
          }}
          ref={(el) => queueMicrotask(() => el.focus())}
        />
        <div class="pal-list" id="pal-list" role="listbox" aria-label="Resultados">
          <For each={shown()} fallback={<div class="pal-empty">Nada encontrado</div>}>
            {(i, idx) => (
              <div class="pal-item" id={`pal-${idx()}`} role="option" aria-selected={idx() === sel()} classList={{ on: idx() === sel() }} onMouseEnter={() => setSel(idx())} onClick={() => choose(i)}>
                <span>{i.label}</span>
                <span class="pal-hint">{i.hint}</span>
              </div>
            )}
          </For>
        </div>
      </Modal>
    </Show>
  );
}
