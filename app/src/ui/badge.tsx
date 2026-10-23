import { cva, type VariantProps } from "class-variance-authority";
import { JSX, splitProps } from "solid-js";
import { cn } from "./cn";

export const badgeVariants = cva(
  "inline-flex items-center gap-1 whitespace-nowrap border leading-none tabular-nums",
  {
    variants: {
      variant: {
        /** a small tag: Docker states, ports, PR facts */
        default: "rounded-xl border-[var(--line-2)] bg-secondary px-1.5 py-[3px] text-[length:var(--fs-xs)] text-muted-foreground",
        agent: "rounded-xl border-[var(--line-3)] bg-[var(--accent-bg)] px-1.5 py-[3px] text-[length:var(--fs-xs)] text-primary",
        high: "rounded-xl border-[var(--line-3)] bg-[var(--warn-bg)] px-1.5 py-[3px] text-[length:var(--fs-xs)] text-[var(--orange)]",
        id: "ml-auto max-w-[110px] overflow-hidden text-ellipsis rounded-xl border-transparent bg-transparent px-1.5 py-[3px] font-mono text-[length:var(--fs-xs)] text-muted-foreground",
        warn: "rounded-xl border-[color-mix(in_srgb,var(--warn)_40%,transparent)] bg-secondary px-1.5 py-[3px] text-[length:var(--fs-xs)] text-[var(--warn)]",
        bad: "rounded-xl border-destructive bg-secondary px-1.5 py-[3px] text-[length:var(--fs-xs)] text-destructive",
        /** a count or note beside a name: no box */
        quiet: "ml-1 flex-none border-transparent bg-transparent text-xs text-muted-foreground",
        overlap: "ml-1 flex-none border-transparent bg-transparent text-xs text-[var(--warn)]",
        /** a pull request number; the colour follows the CI result */
        pr: "ml-1 flex-none rounded-lg border-[var(--line-3)] bg-transparent px-[5px] text-xs text-muted-foreground",
        prOk: "ml-1 flex-none rounded-lg border-[color-mix(in_srgb,var(--ok)_40%,transparent)] bg-transparent px-[5px] text-xs text-[var(--ok)]",
        prErr: "ml-1 flex-none rounded-lg border-[color-mix(in_srgb,var(--err)_45%,transparent)] bg-transparent px-[5px] text-xs text-[var(--err)]",
        prPend: "ml-1 flex-none rounded-lg border-[color-mix(in_srgb,var(--warn)_45%,transparent)] bg-transparent px-[5px] text-xs text-[var(--warn)]",
      },
    },
    defaultVariants: { variant: "default" },
  },
);

export type BadgeProps = JSX.HTMLAttributes<HTMLSpanElement> & VariantProps<typeof badgeVariants>;

export function Badge(props: BadgeProps) {
  const [own, rest] = splitProps(props, ["variant", "class"]);
  return <span {...rest} class={cn(badgeVariants({ variant: own.variant }), own.class)} />;
}
