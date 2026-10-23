import { JSX, splitProps } from "solid-js";
import { cn } from "./cn";

/** A native checkbox with its label beside it: keyboard, form and screen-reader behaviour stay the browser's. */
export function Checkbox(props: Omit<JSX.InputHTMLAttributes<HTMLInputElement>, "type" | "children"> & { label: JSX.Element; labelClass?: string }) {
  const [own, rest] = splitProps(props, ["label", "labelClass", "class"]);
  return (
    <label class={cn("inline-flex cursor-pointer items-center gap-2 text-sm text-foreground", own.labelClass)}>
      <input {...rest} type="checkbox" class={cn("size-3.5 cursor-pointer accent-primary", own.class)} /> {own.label}
    </label>
  );
}
