import { RadioGroup as K } from "@kobalte/core/radio-group";
import { For } from "solid-js";
import { nextRadio } from "../a11y";
import { cn } from "./cn";

export type RadioOption<T extends string> = { value: T; label: string; hint?: string };

/** A row of exclusive choices on Kobalte's RadioGroup: native radios under a `radiogroup`, one tab stop (the checked
 *  one). Arrow / Home / End move focus and select, and wrap around, like `radioKeys` in a11y.ts. */
export function RadioGroup<T extends string>(props: { label: string; value: T; options: RadioOption<T>[]; onChange: (v: T) => void; class?: string }) {
  const onKeyDown = (e: KeyboardEvent & { currentTarget: HTMLElement }) => {
    const radios = Array.from(e.currentTarget.querySelectorAll<HTMLInputElement>('input[type="radio"]'));
    const at = radios.indexOf(document.activeElement as HTMLInputElement);
    const to = nextRadio(at < 0 ? 0 : at, radios.length, e.key);
    if (to === null) return;
    e.preventDefault();
    radios[to].focus();
    radios[to].click();
  };
  return (
    <K value={props.value} onChange={(v) => props.onChange(v as T)} aria-label={props.label} onKeyDown={onKeyDown} class={cn("flex flex-wrap gap-1.5", props.class)}>
      <For each={props.options}>
        {(o) => (
          <K.Item value={o.value} title={o.hint} class="relative rounded-full has-[:focus-visible]:ring-2 has-[:focus-visible]:ring-ring/60">
            <K.ItemInput />
            <K.ItemLabel class="inline-flex h-7 cursor-pointer select-none items-center rounded-full border border-border px-3 text-sm text-foreground transition-colors hover:border-primary data-[checked]:border-primary data-[checked]:bg-primary/10 data-[checked]:text-primary">
              {o.label}
            </K.ItemLabel>
          </K.Item>
        )}
      </For>
    </K>
  );
}
