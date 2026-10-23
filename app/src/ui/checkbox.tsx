import { JSX, splitProps } from "solid-js";
import { cn } from "./cn";

/** Native checkbox (keeps keyboard and form behaviour) painted with the primary colour. */
export function Checkbox(props: Omit<JSX.InputHTMLAttributes<HTMLInputElement>, "type">) {
  const [own, rest] = splitProps(props, ["class"]);
  return <input {...rest} type="checkbox" class={cn("size-3.5 cursor-pointer align-middle accent-primary disabled:opacity-50", own.class)} />;
}
