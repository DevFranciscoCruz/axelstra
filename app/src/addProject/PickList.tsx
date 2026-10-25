import { For, JSX, createEffect, createSignal } from "solid-js";
import { cn } from "../ui/cn";
import { listKey } from "./logic";

/** Highlight and keys of a list that is picked from: shared by the list itself and by a filter input above it. */
export function createPickNav<T>(items: () => T[], activate: (item: T) => void) {
  const [row, setRow] = createSignal(-1);
  // the list changed under the highlight (a filter, a refresh): start again from the top
  createEffect(() => {
    items();
    setRow(-1);
  });
  /** true when the key was a list key (the caller then stops it from moving a caret or scrolling the dialog) */
  const onKey = (e: KeyboardEvent): boolean => {
    const list = items();
    const { row: next, activate: go } = listKey(row(), list.length, e.key);
    if (e.key === "Enter") {
      if (!go) return false;
      activate(list[row()]);
      return true;
    }
    if (next === row() && !["ArrowDown", "ArrowUp", "Home", "End"].includes(e.key)) return false;
    setRow(next);
    return true;
  };
  return { row, setRow, onKey };
}
export type PickNav<T> = ReturnType<typeof createPickNav<T>>;

/** A listbox of options: arrows, Home/End and Enter through `nav`, a click picks. `disabled` keeps it visible but inert. */
export function PickList<T>(props: {
  id: string;
  label: string;
  items: T[];
  nav: PickNav<T>;
  disabled?: boolean;
  class?: string;
  children: (item: T, index: number) => JSX.Element;
  onPick: (item: T) => void;
}) {
  return (
    <ul
      id={props.id}
      role="listbox"
      tabindex="0"
      aria-label={props.label}
      aria-disabled={props.disabled}
      aria-activedescendant={props.nav.row() >= 0 ? `${props.id}-${props.nav.row()}` : undefined}
      class={cn(
        "m-0 max-h-60 min-h-24 list-none overflow-auto rounded-md border border-border p-1 outline-none focus-visible:ring-2 focus-visible:ring-ring/60",
        props.disabled && "pointer-events-none opacity-60",
        props.class,
      )}
      onKeyDown={(e) => {
        if (!props.disabled && props.nav.onKey(e)) e.preventDefault();
      }}
    >
      <For each={props.items}>
        {(item, i) => (
          <li
            id={`${props.id}-${i()}`}
            role="option"
            aria-selected={props.nav.row() === i()}
            ref={(el) => createEffect(() => props.nav.row() === i() && el.scrollIntoView?.({ block: "nearest" }))}
            class="flex cursor-pointer flex-col gap-0.5 rounded-sm px-2 py-1 text-base hover:bg-accent aria-selected:bg-accent"
            onClick={() => {
              props.nav.setRow(i());
              if (!props.disabled) props.onPick(item);
            }}
          >
            {props.children(item, i())}
          </li>
        )}
      </For>
    </ul>
  );
}
