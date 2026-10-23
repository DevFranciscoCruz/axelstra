import { Show, createSignal } from "solid-js";
import { Icon } from "../Icon";
import { menuOpen, showListboxAt } from "../ContextMenu";
import { cn } from "./cn";

export type Option = { value: string; label: string; icon?: string };

/** Dropdown that matches the app instead of the system list. The list is the app's listbox layer (ContextMenu), so Esc and
 *  Tab close it before they reach a dialog underneath. */
export function Select(props: { value: string; options: Option[]; onChange: (v: string) => void; class?: string; label?: string }) {
  const label = () => props.options.find((o) => o.value === props.value)?.label ?? props.value;
  const icon = () => props.options.find((o) => o.value === props.value)?.icon;
  const [mine, setMine] = createSignal(false);
  return (
    <button
      type="button"
      class={cn(
        "select-btn flex w-full cursor-pointer items-center justify-between gap-2 rounded-md border border-input bg-background py-1.5 pr-2 pl-2.5 text-left text-base text-foreground outline-none hover:border-primary focus-visible:ring-2 focus-visible:ring-ring/40 disabled:opacity-50",
        props.class,
      )}
      aria-haspopup="listbox"
      aria-expanded={mine() && menuOpen()}
      aria-label={props.label ? `${props.label}: ${label()}` : undefined}
      onClick={(e) => {
        e.stopPropagation();
        const r = e.currentTarget.getBoundingClientRect();
        setMine(true);
        showListboxAt(
          r.left,
          r.bottom + 4,
          props.options.map((o) => ({ label: o.label, icon: o.icon ?? (o.value === props.value ? "check" : ""), selected: o.value === props.value, run: () => props.onChange(o.value) })),
          Math.max(r.width, 140),
          () => setMine(false),
        );
      }}
    >
      <Show when={icon()}>{(n) => <Icon name={n()} size={13} />}</Show>
      <span class="select-label min-w-0 flex-1 truncate">{label()}</span>
      <span class="chev text-muted-foreground"><Icon name="chevDown" size={11} /></span>
    </button>
  );
}
