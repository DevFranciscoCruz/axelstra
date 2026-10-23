import { Checkbox as K } from "@kobalte/core/checkbox";
import { JSX } from "solid-js";
import { cn } from "./cn";

/** A checkbox with its label. `onChange` gets the new state. */
export function CheckboxField(props: { checked: boolean; onChange: (v: boolean) => void; class?: string; children: JSX.Element }) {
  return (
    <K checked={props.checked} onChange={props.onChange} class={cn("flex cursor-pointer items-start gap-2 text-sm text-foreground", props.class)}>
      <K.Input />
      <K.Control class="mt-0.5 flex size-3.5 shrink-0 items-center justify-center rounded-sm border border-input bg-[var(--s-in)] data-[checked]:border-primary data-[checked]:bg-primary data-[focus-visible]:ring-2 data-[focus-visible]:ring-ring/60">
        <K.Indicator class="text-primary-foreground">
          <svg viewBox="0 0 12 12" class="size-2.5" fill="none" stroke="currentColor" stroke-width="2" aria-hidden="true">
            <path d="M2 6.5l2.5 2.5L10 3.5" />
          </svg>
        </K.Indicator>
      </K.Control>
      <K.Label class="cursor-pointer">{props.children}</K.Label>
    </K>
  );
}
