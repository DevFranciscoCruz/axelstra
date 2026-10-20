import { For } from "solid-js";
import { showMenuAt } from "./ContextMenu";

export type Option = { value: string; label: string };

/** Dropdown that matches the app instead of the system list. */
export function Select(props: { value: string; options: Option[]; onChange: (v: string) => void; class?: string }) {
  const label = () => props.options.find((o) => o.value === props.value)?.label ?? props.value;
  return (
    <button
      type="button"
      class={`select-btn ${props.class ?? ""}`}
      onClick={(e) => {
        e.stopPropagation();
        const r = e.currentTarget.getBoundingClientRect();
        showMenuAt(
          r.left,
          r.bottom + 4,
          props.options.map((o) => ({ label: o.label, icon: o.value === props.value ? "✓" : "", run: () => props.onChange(o.value) })),
          Math.max(r.width, 140),
        );
      }}
    >
      <span class="select-label">{label()}</span>
      <span class="chev">▾</span>
    </button>
  );
}

export { For };
