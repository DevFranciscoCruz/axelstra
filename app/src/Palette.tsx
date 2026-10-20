import { For, Show, createEffect, createMemo, createSignal, onCleanup, onMount } from "solid-js";
import { projects, selectWorktree, setRightTab, setShowAutomations, setShowKanban } from "./store";

export type PaletteAction = { label: string; hint?: string; run: () => void };

type Item = { label: string; hint: string; run: () => void };

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
    out.push(
      { label: "Ficheiros", hint: "painel", run: () => setRightTab("files") },
      { label: "Alterações", hint: "painel", run: () => setRightTab("changes") },
      { label: "PRs e issues", hint: "painel", run: () => setRightTab("pulls") },
      { label: "Kanban", hint: "vista", run: () => setShowKanban(true) },
      { label: "Automatizações", hint: "agendamento", run: () => setShowAutomations(true) },
    );
    for (const p of projects()) {
      for (const w of p.worktrees) {
        const branch = w.is_main ? (p.default_branch ?? "main") : (w.branch ?? "(detached)");
        out.push({ label: `${p.name} / ${branch}`, hint: "worktree", run: () => selectWorktree(w.path) });
      }
    }
    return out;
  });

  const shown = createMemo(() => {
    const words = q().toLowerCase().split(/\s+/).filter(Boolean);
    return items().filter((i) => words.every((w) => i.label.toLowerCase().includes(w))).slice(0, 12);
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

  return (
    <Show when={open()}>
      <div class="pal-back" onClick={() => setOpen(false)}>
        <div class="pal" onClick={(e) => e.stopPropagation()}>
          <input
            class="pal-input"
            role="combobox"
            aria-expanded="true"
            aria-controls="pal-list"
            aria-activedescendant={`pal-${sel()}`}
            aria-label="Paleta de comandos"
            placeholder="Ir para worktree ou executar ação…"
            value={q()}
            onInput={(e) => setQ(e.currentTarget.value)}
            onKeyDown={(e) => {
              if (e.key === "Escape") setOpen(false);
              else if (e.key === "ArrowDown") {
                e.preventDefault();
                setSel(Math.min(sel() + 1, shown().length - 1));
              } else if (e.key === "ArrowUp") {
                e.preventDefault();
                setSel(Math.max(sel() - 1, 0));
              } else if (e.key === "Enter") choose(shown()[sel()]);
            }}
            ref={(el) => queueMicrotask(() => el.focus())}
          />
          <div class="pal-list" id="pal-list" role="listbox">
            <For each={shown()} fallback={<div class="pal-empty">Nada encontrado</div>}>
              {(i, idx) => (
                <div class="pal-item" id={`pal-${idx()}`} role="option" aria-selected={idx() === sel()} classList={{ on: idx() === sel() }} onMouseEnter={() => setSel(idx())} onClick={() => choose(i)}>
                  <span>{i.label}</span>
                  <span class="pal-hint">{i.hint}</span>
                </div>
              )}
            </For>
          </div>
        </div>
      </div>
    </Show>
  );
}
