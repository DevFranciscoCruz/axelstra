import { JSX, Show } from "solid-js";

/** A label above a control. */
export function Field(props: { label: string; hint?: string; full?: boolean; children: JSX.Element }) {
  return (
    <label classList={{ "m-full": props.full }}>
      {props.label}
      {props.children}
      <Show when={props.hint}>{(h) => <span class="auto-note">{h()}</span>}</Show>
    </label>
  );
}
