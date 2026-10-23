import { cva, type VariantProps } from "class-variance-authority";
import { JSX, splitProps } from "solid-js";
import { cn } from "./cn";

export const buttonVariants = cva(
  "inline-flex items-center justify-center gap-1.5 whitespace-nowrap rounded-md border text-sm font-medium cursor-pointer select-none transition-colors outline-none focus-visible:ring-2 focus-visible:ring-ring/60 disabled:pointer-events-none disabled:opacity-50 [&_svg]:shrink-0",
  {
    variants: {
      variant: {
        default: "border-primary bg-primary text-primary-foreground hover:bg-primary/90",
        outline: "border-border bg-transparent text-foreground hover:border-primary hover:text-primary",
        secondary: "border-transparent bg-secondary text-secondary-foreground hover:bg-secondary/80",
        ghost: "border-transparent bg-transparent text-foreground hover:bg-accent",
        destructive: "border-destructive bg-destructive text-destructive-foreground hover:bg-destructive/90",
      },
      size: { default: "h-8 px-3", sm: "h-7 px-2.5", icon: "size-7 p-0" },
    },
    defaultVariants: { variant: "outline", size: "default" },
  },
);

export type ButtonProps = JSX.ButtonHTMLAttributes<HTMLButtonElement> & VariantProps<typeof buttonVariants>;

export function Button(props: ButtonProps) {
  const [own, rest] = splitProps(props, ["variant", "size", "class"]);
  return <button type="button" {...rest} class={cn(buttonVariants({ variant: own.variant, size: own.size }), own.class)} />;
}
