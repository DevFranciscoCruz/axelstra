import { Tabs as K } from "@kobalte/core/tabs";
import { cva, type VariantProps } from "class-variance-authority";
import { ComponentProps, splitProps } from "solid-js";
import { cn } from "./cn";

/** Tabs on Kobalte: tablist/tab roles, a roving tabindex and arrow / Home / End navigation come from the library.
 *  `Tabs` is the root (`value`, `onChange`, `activationMode`: "automatic" selects as you arrow, "manual" only moves focus). */
export const Tabs = K;

export const tabsListVariants = cva("", {
  variants: {
    variant: {
      /** a strip of tabs along a bar */
      bar: "flex items-stretch gap-0.5",
      /** a two-way switch inside a panel */
      segmented: "grid grid-cols-2 rounded-md border border-[var(--line-2)] bg-background p-0.5",
    },
  },
  defaultVariants: { variant: "bar" },
});

export const tabsTriggerVariants = cva(
  "cursor-pointer border-0 bg-transparent font-[inherit] outline-none focus-visible:outline-2 focus-visible:-outline-offset-1 focus-visible:outline-ring",
  {
    variants: {
      variant: {
        /** a tab of a bar: the selected one takes the page colour */
        bar: "flex-none rounded-t-md px-2.5 py-1.5 text-muted-foreground hover:text-foreground data-[selected]:bg-background data-[selected]:text-foreground",
        /** the button inside a `Tab` wrapper, which draws the tab itself */
        bare: "flex min-w-0 items-center gap-1.5 whitespace-nowrap p-0 text-inherit [&>span:not(.dot):not(.dtag)]:truncate",
        segmented: "rounded-sm py-[3px] text-sm text-muted-foreground data-[selected]:bg-popover data-[selected]:text-foreground data-[selected]:shadow-[0_0_0_1px_var(--line-3)]",
      },
    },
    defaultVariants: { variant: "bar" },
  },
);

export function TabsList(props: ComponentProps<typeof K.List> & VariantProps<typeof tabsListVariants>) {
  const [own, rest] = splitProps(props, ["variant", "class"]);
  return <K.List {...rest} class={cn(tabsListVariants({ variant: own.variant }), own.class)} />;
}

/** `selected` overrides what Kobalte derives from the single `value` (a strip where several tabs can look selected). */
export function TabsTrigger(props: ComponentProps<typeof K.Trigger> & VariantProps<typeof tabsTriggerVariants> & { selected?: boolean }) {
  const [own, rest] = splitProps(props, ["variant", "class", "selected"]);
  return (
    <K.Trigger
      {...rest}
      {...(own.selected === undefined ? {} : { "aria-selected": own.selected, "data-selected": own.selected ? "" : undefined })}
      class={cn(tabsTriggerVariants({ variant: own.variant }), own.class)}
    />
  );
}
