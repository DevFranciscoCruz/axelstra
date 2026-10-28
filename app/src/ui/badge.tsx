import { cva, type VariantProps } from "class-variance-authority";
import { JSX, splitProps } from "solid-js";
import { cn } from "./cn";

/** the box shared by the tag-like variants */
const TAG = "rounded-xl px-1.5 py-[3px] text-[length:var(--fs-xs)]";
/** the small outlined pull request number */
const PR = "ml-1 flex-none rounded-lg bg-transparent px-[5px] text-xs";

export const badgeVariants = cva("inline-flex items-center gap-1 whitespace-nowrap border leading-none tabular-nums", {
  variants: {
    variant: {
      /** a small tag: Docker states, ports, PR facts */
      default: `${TAG} border-[var(--line-2)] bg-secondary text-muted-foreground`,
      agent: `${TAG} border-[var(--line-3)] bg-[var(--accent-bg)] text-primary`,
      high: `${TAG} border-[var(--line-3)] bg-[var(--warn-bg)] text-[var(--orange)]`,
      id: `${TAG} ml-auto max-w-[110px] overflow-hidden text-ellipsis border-transparent bg-transparent font-mono text-muted-foreground`,
      warn: `${TAG} border-[var(--warn-line)] bg-secondary text-[var(--warn)]`,
      bad: `${TAG} border-destructive bg-secondary text-destructive`,
      /** a count or note beside a name: no box */
      quiet: "ml-1 flex-none border-transparent bg-transparent text-xs text-muted-foreground",
      overlap: "ml-1 flex-none border-transparent bg-transparent text-xs text-[var(--warn)]",
      /** a pull request number; the colour follows the CI result */
      pr: `${PR} border-[var(--line-3)] text-muted-foreground`,
      prOk: `${PR} border-[var(--ok-line)] text-[var(--ok)]`,
      prErr: `${PR} border-[var(--err-line)] text-[var(--danger)]`,
      prPend: `${PR} border-[var(--warn-line)] text-[var(--warn)]`,
    },
  },
  defaultVariants: { variant: "default" },
});

export type BadgeProps = JSX.HTMLAttributes<HTMLSpanElement> & VariantProps<typeof badgeVariants>;

export function Badge(props: BadgeProps) {
  const [own, rest] = splitProps(props, ["variant", "class"]);
  return <span {...rest} class={cn(badgeVariants({ variant: own.variant }), own.class)} />;
}
