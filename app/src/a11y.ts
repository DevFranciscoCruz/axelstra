import type { JSX } from "solid-js";

/** Makes a clickable div/span reachable and operable from the keyboard. */
export const press = (fn: (e: Event) => void, role: "button" | "tab" | "option" = "button"): JSX.HTMLAttributes<any> => ({
  role,
  tabindex: 0,
  onClick: fn,
  onKeyDown: (e) => {
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

/** Icon-only buttons carry their name in `title`; mirror it to aria-label so screen readers get it.
 * Added nodes are collected and handled once per frame, so a big re-render costs one pass. */
export const labelIcons = () => {
  const sel = "button[title]:not([aria-label])";
  const label = (b: HTMLElement) => b.setAttribute("aria-label", b.title);
  document.querySelectorAll<HTMLElement>(sel).forEach(label);
  const queue = new Set<HTMLElement>();
  let scheduled = false;
  const run = () => {
    scheduled = false;
    for (const n of queue) {
      if (!n.isConnected) continue;
      if (n.matches(sel)) label(n);
      n.querySelectorAll<HTMLElement>(sel).forEach(label);
    }
    queue.clear();
  };
  new MutationObserver((ms) => {
    for (const m of ms) m.addedNodes.forEach((n) => n instanceof HTMLElement && queue.add(n));
    if (queue.size && !scheduled) {
      scheduled = true;
      requestAnimationFrame(run);
    }
  }).observe(document.body, { childList: true, subtree: true });
};
