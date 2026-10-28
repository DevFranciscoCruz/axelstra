import "./shell.css";
import { For, Show, createEffect, createMemo, createSignal, onCleanup, onMount } from "solid-js";
import { moveIndex } from "./a11y";
import { Modal } from "./Modal";
import { createPersisted, isStringArray } from "./persist";
import { pushRecent, rank } from "./paletteLogic";
import { projects, selectWorktree } from "./store";
import { S } from "./strings";
import { Input } from "./ui";

export type PaletteAction = { id?: string; label: string; hint?: string; run: () => void };

type Item = { id: string; label: string; hint: string; run: () => void };

/** more than this many matches is noise; typing narrows it */
const MAX_SHOWN = 200;

const [recents, setRecents] = createPersisted<string[]>("axelstra.palette.recents", isStringArray, []);

/** Ctrl/Cmd+K: jump to any worktree or run a command from one search box */
export function Palette(props: { actions: () => PaletteAction[] }) {
  const [open, setOpen] = createSignal(false);
  const [q, setQ] = createSignal("");
  const [sel, setSel] = createSignal(0);
  // Enter on an empty result list says so instead of doing nothing
  const [nothing, setNothing] = createSignal(false);
  // the pointer selects a row only when it moves: rows sliding under a still pointer (scrolling, arrowing) do not
  let lastPoint = { x: -1, y: -1 };

  onMount(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        e.stopPropagation();
        setQ("");
        setSel(0);
        setNothing(false);
        setOpen(!open());
      }
    };
    // captured, like Ctrl+B: the terminal must never swallow it
    window.addEventListener("keydown", onKey, true);
    onCleanup(() => window.removeEventListener("keydown", onKey, true));
  });

  const items = createMemo<Item[]>(() => {
    const out: Item[] = props.actions().map((a) => ({ id: a.id ?? a.label, label: a.label, hint: a.hint ?? S.paletteAction, run: a.run }));
    for (const p of projects()) {
      for (const w of p.worktrees) {
        const branch = w.is_main ? (p.default_branch ?? "main") : (w.branch ?? S.detached);
        out.push({ id: `wt:${w.path}`, label: `${p.name} / ${branch}`, hint: S.paletteWorktree, run: () => selectWorktree(w.path) });
      }
    }
    return out;
  });

  const shown = createMemo(() => rank(items(), q(), recents(), MAX_SHOWN));

  createEffect(() => {
    shown();
    setSel(0);
    setNothing(false);
  });
  // keep the highlighted row visible while arrowing through a long list
  createEffect(() => {
    sel();
    queueMicrotask(() => document.querySelector(".pal-item.on")?.scrollIntoView({ block: "nearest" }));
  });

  const choose = (i: Item | undefined) => {
    if (!i) {
      setNothing(true);
      return;
    }
    setOpen(false);
    setRecents((r) => pushRecent(r, i.id));
    i.run();
  };

  // a modal dialog: Esc closes only this one (the dialog underneath stays), Tab stays inside, focus goes back on close
  return (
    <Show when={open()}>
      <Modal label={S.paletteTitle} onClose={() => setOpen(false)} backClass="pal-back" class="pal">
        <Input
          class="pal-input"
          role="combobox"
          aria-expanded={shown().length > 0}
          aria-controls="pal-list"
          aria-activedescendant={shown().length ? `pal-${sel()}` : undefined}
          aria-label={S.paletteTitle}
          placeholder={S.palettePlaceholder}
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
        <div class="sr-only" role="status" aria-live="polite" aria-atomic="true">
          {shown().length ? S.paletteCount(shown().length) : ""}
        </div>
        <div class="pal-list" id="pal-list" role="listbox" aria-label={S.paletteResults}>
          <For each={shown()} fallback={<div class="pal-empty" role="status" classList={{ "text-destructive": nothing() }}>{nothing() ? S.paletteNoMatch(q()) : S.paletteEmpty}</div>}>
            {(i, idx) => (
              <div
                class="pal-item"
                id={`pal-${idx()}`}
                role="option"
                aria-selected={idx() === sel()}
                classList={{ on: idx() === sel() }}
                onMouseMove={(e) => {
                  if (e.clientX === lastPoint.x && e.clientY === lastPoint.y) return;
                  lastPoint = { x: e.clientX, y: e.clientY };
                  setSel(idx());
                }}
                onClick={() => choose(i)}
              >
                <span>{i.label}</span>
                <span class="pal-hint">{i.hint}</span>
              </div>
            )}
          </For>
        </div>
        <div class="pal-foot" aria-hidden="true">{S.paletteFoot}</div>
      </Modal>
    </Show>
  );
}
