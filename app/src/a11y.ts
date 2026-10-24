import type { JSX } from "solid-js";

/** Makes a clickable div/span reachable and operable from the keyboard. `extra` sees every key first (a row's own shortcuts, its menu key) so an explicit onKeyDown does not replace Enter/Space. */
export const press = (fn: (e: Event) => void, role: "button" | "tab" | "option" = "button", extra?: (e: KeyboardEvent & { currentTarget: HTMLElement }) => void): JSX.HTMLAttributes<any> => ({
  role,
  tabindex: 0,
  onClick: fn,
  onKeyDown: (e) => {
    extra?.(e);
    if (e.defaultPrevented) return;
    if (e.target !== e.currentTarget) return;
    if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      fn(e);
    }
  },
});

/** Index of the radio that an arrow / Home / End key moves to, or null when the key does nothing. Wraps around. */
export const nextRadio = (current: number, count: number, key: string): number | null => {
  if (count <= 0) return null;
  switch (key) {
    case "ArrowRight":
    case "ArrowDown":
      return (current + 1) % count;
    case "ArrowLeft":
    case "ArrowUp":
      return (current - 1 + count) % count;
    case "Home":
      return 0;
    case "End":
      return count - 1;
    default:
      return null;
  }
};

/** Roving tabindex for a `role=radiogroup`: arrow keys move focus and select, only the checked radio is in the tab order.
 * Spread on the group (`onKeyDown`) and give each radio `tabindex={checked ? 0 : -1}`. */
export const radioKeys = (e: KeyboardEvent & { currentTarget: HTMLElement }) => {
  const radios = Array.from(e.currentTarget.querySelectorAll<HTMLElement>('[role="radio"]'));
  const at = radios.indexOf(document.activeElement as HTMLElement);
  const to = nextRadio(at < 0 ? 0 : at, radios.length, e.key);
  if (to === null) return;
  e.preventDefault();
  radios[to].focus();
  radios[to].click();
};

/** Row a listbox moves to for an arrow / Home / End key (stays at the ends), or null when the key does nothing. */
export const moveIndex = (current: number, count: number, key: string): number | null => {
  if (count <= 0) return null;
  switch (key) {
    case "ArrowDown":
      return Math.min(current + 1, count - 1);
    case "ArrowUp":
      return Math.max(current - 1, 0);
    case "Home":
      return 0;
    case "End":
      return count - 1;
    default:
      return null;
  }
};

/** Roving focus for a `role=tablist`: arrows, Home and End move focus between the tabs (wrapping), Enter or Space then picks one.
 * Spread on the list (`onKeyDown`); give each tab `tabindex={selected ? 0 : -1}`. Pass `select` for tabs that switch as you arrow. */
export const tabKeys = (e: KeyboardEvent & { currentTarget: HTMLElement }, select = false) => {
  const tabs = Array.from(e.currentTarget.querySelectorAll<HTMLElement>('[role="tab"]'));
  const at = tabs.indexOf(document.activeElement as HTMLElement);
  if (at < 0) return;
  const to = nextRadio(at, tabs.length, e.key === "ArrowUp" || e.key === "ArrowDown" ? "" : e.key);
  if (to === null) return;
  e.preventDefault();
  tabs[to].focus();
  if (select) tabs[to].click();
};
