import { Show, createSignal } from "solid-js";
import { Icon } from "./Icon";
import { menuOpen, showListboxAt } from "./ContextMenu";

export type Option = { value: string; label: string; icon?: string };

/** Dropdown that matches the app instead of the system list. */
export function Select(props: { value: string; options: Option[]; onChange: (v: string) => void; class?: string; label?: string }) {
  const label = () => props.options.find((o) => o.value === props.value)?.label ?? props.value;
  const icon = () => props.options.find((o) => o.value === props.value)?.icon;
  const [mine, setMine] = createSignal(false);
  return (
    <button
      type="button"
      class={`select-btn ${props.class ?? ""}`}
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
      <span class="select-label">{label()}</span>
      <span class="chev"><Icon name="chevDown" size={11} /></span>
    </button>
  );
}
