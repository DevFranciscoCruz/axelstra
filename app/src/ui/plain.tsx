import { JSX } from "solid-js";

/** A button that keeps the look of the class it is given (`icon`, `sb-item`, `wt-open`...): rows, chips, icon buttons and
 *  other controls that are not a standard Button. Only the type and the keyboard focus ring come from here. */
export function PlainButton(props: JSX.ButtonHTMLAttributes<HTMLButtonElement>) {
  return <button type="button" {...props} />;
}

/** An input that keeps the look of the class it is given (an inline rename field, for example). */
export function PlainInput(props: JSX.InputHTMLAttributes<HTMLInputElement>) {
  return <input {...props} />;
}
