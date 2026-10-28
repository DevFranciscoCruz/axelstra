import { cva, type VariantProps } from "class-variance-authority";
import { JSX, splitProps } from "solid-js";
import { cn } from "./cn";

export const buttonVariants = cva(
  "inline-flex items-center justify-center gap-1.5 whitespace-nowrap rounded-md border text-sm font-medium cursor-pointer select-none transition-colors outline-hidden focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-50 [&_svg]:shrink-0",
  {
    variants: {
      variant: {
        default: "border-primary bg-primary text-primary-foreground hover:bg-primary/90",
        outline: "border-border bg-transparent text-foreground hover:border-primary hover:text-primary",
        secondary: "border-transparent bg-secondary text-secondary-foreground hover:bg-secondary/80",
        ghost: "border-transparent bg-transparent text-foreground hover:bg-accent",
        destructive: "border-destructive bg-destructive text-destructive-foreground hover:bg-destructive/90",
        /** a bare glyph button (toolbar, close, fold): muted, lights up on hover */
        icon: "border-transparent bg-transparent font-normal text-muted-foreground hover:bg-secondary hover:text-foreground",
        /** a clickable list row: takes the colour and font of its parent, left aligned */
        row: "border-transparent bg-transparent font-normal text-inherit text-[length:inherit] justify-start text-left whitespace-normal",
        /** a small outlined tag-like button */
        chip: "border-border bg-transparent font-normal text-muted-foreground hover:border-primary hover:text-primary",
        /** muted text that darkens on hover (side links, section toggles) */
        link: "border-transparent bg-transparent font-normal text-muted-foreground hover:text-foreground",
        /** a tinted success button (an open port) */
        success: "border-[var(--ok-line)] bg-[var(--ok-tint)] font-normal text-[var(--ok)] hover:bg-[var(--ok-tint-hover)]",
      },
      size: {
        default: "h-8 px-3",
        sm: "h-7 px-2.5",
        icon: "size-7 p-0",
        /** square toolbar button (tab bar) */
        tool: "size-[26px] p-0",
        /** a text glyph button */
        glyph: "h-auto rounded-sm px-1.5 py-0.5 text-xl leading-none",
        /** no box of its own: the parent decides the size */
        bare: "h-auto p-0",
        /** a status-bar item */
        xs: "h-[18px] gap-1 rounded-sm px-1.5",
        chip: "h-auto rounded-lg px-2 py-px text-xs",
        pill: "h-auto rounded-full px-3 py-1",
        /** the two halves of a split button: the action and its menu */
        split: "h-auto flex-1 gap-1.5 rounded-l-lg rounded-r-none p-2",
        splitEnd: "h-auto w-[34px] rounded-l-none rounded-r-lg border-l-0 p-0",
      },
    },
    defaultVariants: { variant: "outline", size: "default" },
  },
);

export type ButtonProps = JSX.ButtonHTMLAttributes<HTMLButtonElement> & VariantProps<typeof buttonVariants>;

export function Button(props: ButtonProps) {
  const [own, rest] = splitProps(props, ["variant", "size", "class"]);
  return <button type="button" {...rest} class={cn(buttonVariants({ variant: own.variant, size: own.size }), own.class)} />;
}
