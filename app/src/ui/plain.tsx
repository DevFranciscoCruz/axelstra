import { JSX } from "solid-js";

/** Only ContextMenu still uses this (its rows keep the `ctx-item` class). Remove it when ContextMenu moves to Button. */
export function PlainButton(props: JSX.ButtonHTMLAttributes<HTMLButtonElement>) {
  return <button type="button" {...props} />;
}
