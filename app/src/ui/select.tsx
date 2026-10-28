import { Select as K } from "@kobalte/core/select";
import { Show, createEffect, createSignal, onCleanup } from "solid-js";
import { Icon } from "../Icon";
import { holdLayer } from "../ContextMenu";
import { cn } from "./cn";

/** agent logos wear their brand colour through `data-agent` (tokens `--brand-*` in styles.css, one set per theme);
 *  the CSP forbids `style=`. Other icons follow the text colour. */
const agentAttr = (icon: string | undefined) =>
  icon === "claude" || icon === "codex" || icon === "gemini" ? icon : undefined;

export type Option = {
  value: string;
  label: string;
  icon?: string;
  /** second line under the label in the list */ desc?: string;
};

/** Dropdown that matches the app instead of the system list, on Kobalte's Select (listbox, roving focus, type-ahead).
 *  While it is open it holds a layer (ContextMenu's `menuOpen`), so Esc and Tab close it before they reach a dialog
 *  underneath. */
export function Select(props: {
  value: string;
  options: Option[];
  onChange: (v: string) => void;
  class?: string;
  label?: string;
  /** options that have an icon show only the icon (the name stays as tooltip and for screen readers) */ iconOnly?: boolean;
}) {
  const current = () => props.options.find((o) => o.value === props.value);
  const label = () => current()?.label ?? props.value;
  const logoOnly = () => !!props.iconOnly && !!current()?.icon;
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
        return (
          <K.Item
            item={ip.item}
            title={props.iconOnly && o().icon ? o().label : undefined}
            class="grid cursor-pointer grid-cols-[auto_minmax(0,1fr)_auto] items-center gap-x-2.5 rounded-md px-2.5 py-1.5 text-base text-popover-foreground outline-hidden data-[highlighted]:bg-accent data-[highlighted]:ring-2 data-[highlighted]:ring-inset data-[highlighted]:ring-ring"
          >
            <Show when={o().icon}>
              {(n) => (
                <span
                  class="inline-flex w-4 flex-none items-center justify-center text-muted-foreground"
                  data-agent={agentAttr(n())}
                >
                  <Icon name={n()} size={13} />
                </span>
              )}
            </Show>
            <span class={cn("flex min-w-0 flex-col", !o().icon && "col-start-1")}>
              <K.ItemLabel class={cn(props.iconOnly && o().icon && "sr-only")}>{o().label}</K.ItemLabel>
              <Show when={o().desc}>
                <K.ItemDescription class="text-xs text-muted-foreground">{o().desc}</K.ItemDescription>
              </Show>
            </span>
            <span class="col-start-3 inline-flex w-4 items-center justify-center text-muted-foreground">
              <Show when={sel()}>
                <Icon name="check" size={13} />
              </Show>
            </span>
          </K.Item>
        );
      }}
    >
      <K.Trigger
        ref={trigger}
        class={cn(
          "select-btn flex w-full cursor-pointer items-center justify-between gap-2 rounded-md border border-input bg-background py-1.5 pr-2 pl-2.5 text-left text-base text-foreground outline-hidden hover:border-primary focus-visible:border-ring focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring disabled:opacity-50",
          props.class,
        )}
        aria-label={props.label ? `${props.label}: ${label()}` : undefined}
        title={logoOnly() ? label() : undefined}
        // a click that did not come from a pointer (assistive tech, scripts) opens it too; pointers open on press
        onClick={(e) => {
          if (!e.currentTarget.dataset.pointerType) setOpen(!open());
        }}
      >
        <Show when={current()?.icon}>
          {(n) => (
            <span class="inline-flex" data-agent={agentAttr(n())}>
              <Icon name={n()} size={logoOnly() ? 16 : 13} />
            </span>
          )}
        </Show>
        <K.Value<Option> class={cn("select-label min-w-0 flex-1 truncate", logoOnly() && "sr-only")}>
          {() => label()}
        </K.Value>
        <K.Icon class="chev text-muted-foreground">
          <Icon name="chevDown" size={11} />
        </K.Icon>
      </K.Trigger>
      <K.Portal>
        <K.Content
          class="z-[70] min-w-[max(var(--kb-popper-anchor-width),8.75rem)] rounded-xl border border-border bg-popover p-1 shadow-lg"
          onContextMenu={(e: MouseEvent) => e.preventDefault()}
        >
          <K.Listbox class="m-0 list-none p-0 outline-hidden" />
        </K.Content>
      </K.Portal>
    </K>
  );
}
