import { JSX, Show } from "solid-js";
import { cn } from "./cn";

/** A label above a control. */
export function Field(props: { label: string; hint?: string; full?: boolean; children: JSX.Element }) {
  return (
    <label class={cn("flex min-w-0 flex-1 flex-col gap-1 text-xs text-muted-foreground", props.full && "m-full")}>
      {props.label}
      {props.children}
      <Show when={props.hint}>{(h) => <span class="auto-note">{h()}</span>}</Show>
    </label>
  );
}
