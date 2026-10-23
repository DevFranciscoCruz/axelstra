import { JSX, splitProps } from "solid-js";
import { cn } from "./cn";

export const inputClass =
  "w-full rounded-sm border border-input bg-[var(--s-in)] px-2 py-1.5 text-base text-foreground outline-none placeholder:text-muted-foreground focus-visible:border-ring focus-visible:ring-2 focus-visible:ring-ring/40 disabled:opacity-50";

export function Input(props: JSX.InputHTMLAttributes<HTMLInputElement>) {
  const [own, rest] = splitProps(props, ["class"]);
  return <input {...rest} class={cn(inputClass, own.class)} />;
}

export function Textarea(props: JSX.TextareaHTMLAttributes<HTMLTextAreaElement>) {
  const [own, rest] = splitProps(props, ["class"]);
  return <textarea {...rest} class={cn(inputClass, "resize-y", own.class)} />;
}
