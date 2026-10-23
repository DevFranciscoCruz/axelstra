import { Select as K } from "@kobalte/core/select";
import { Show, createEffect, createSignal, onCleanup } from "solid-js";
import { Icon, hasIcon } from "../Icon";
import { holdLayer } from "../ContextMenu";
import { cn } from "./cn";

export type Option = { value: string; label: string; icon?: string };

/** Dropdown that matches the app instead of the system list, on Kobalte's Select (listbox, roving focus, type-ahead).
 *  While it is open it holds a layer (ContextMenu's `menuOpen`), so Esc and Tab close it before they reach a dialog
 *  underneath. */
export function Select(props: { value: string; options: Option[]; onChange: (v: string) => void; class?: string; label?: string }) {
  const current = () => props.options.find((o) => o.value === props.value);
  const label = () => current()?.label ?? props.value;
  const [open, setOpen] = createSignal(false);
  let trigger: HTMLButtonElement | undefined;

  createEffect(() => {
    if (!open()) return;
    const release = holdLayer();
    const key = (e: KeyboardEvent) => {
      if (e.key !== "Escape" && e.key !== "Tab") return;
      e.preventDefault();
      setOpen(false);
      trigger?.focus();
    };
    window.addEventListener("keydown", key);
    onCleanup(() => {
      window.removeEventListener("keydown", key);
      release();
    });
  });

  return (
    <K<Option>
      options={props.options}
      optionValue="value"
      optionTextValue="label"
      value={current() ?? null}
      onChange={(o) => o && o.value !== props.value && props.onChange(o.value)}
      open={open()}
      onOpenChange={setOpen}
      disallowEmptySelection
      gutter={4}
      sameWidth={false}
      itemComponent={(ip) => {
        const o = () => ip.item.rawValue;
        const sel = () => o().value === props.value;
        const icon = () => o().icon ?? (sel() ? "check" : "");
        return (
          <K.Item
            item={ip.item}
            class="flex cursor-pointer items-center gap-2.5 rounded-md px-2.5 py-1.5 text-base text-popover-foreground outline-none data-[highlighted]:bg-accent"
          >
            <span class="inline-flex w-4 flex-none items-center justify-center text-muted-foreground">
              {hasIcon(icon()) ? <Icon name={icon()} size={13} /> : icon()}
            </span>
            <K.ItemLabel class="flex-1">{o().label}</K.ItemLabel>
          </K.Item>
        );
      }}
    >
      <K.Trigger
        ref={trigger}
        class={cn(
          "select-btn flex w-full cursor-pointer items-center justify-between gap-2 rounded-md border border-input bg-background py-1.5 pr-2 pl-2.5 text-left text-base text-foreground outline-none hover:border-primary focus-visible:ring-2 focus-visible:ring-ring/40 disabled:opacity-50",
          props.class,
        )}
        aria-label={props.label ? `${props.label}: ${label()}` : undefined}
        // a click that did not come from a pointer (assistive tech, scripts) opens it too; pointers open on press
        onClick={(e) => {
          if (!e.currentTarget.dataset.pointerType) setOpen(!open());
        }}
      >
        <Show when={current()?.icon}>{(n) => <Icon name={n()} size={13} />}</Show>
        <K.Value<Option> class="select-label min-w-0 flex-1 truncate">{() => label()}</K.Value>
        <K.Icon class="chev text-muted-foreground">
          <Icon name="chevDown" size={11} />
        </K.Icon>
      </K.Trigger>
      <K.Portal>
        <K.Content class="z-[70] min-w-[max(var(--kb-popper-anchor-width),8.75rem)] rounded-xl border border-border bg-popover p-1 shadow-lg" onContextMenu={(e: MouseEvent) => e.preventDefault()}>
          <K.Listbox class="outline-none" />
        </K.Content>
      </K.Portal>
    </K>
  );
}
