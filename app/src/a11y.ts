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

/** Icon-only buttons carry their name in `title`; mirror it to aria-label so screen readers get it. */
export const labelIcons = () => {
  const apply = (root: ParentNode) =>
    root.querySelectorAll<HTMLElement>("button[title]:not([aria-label])").forEach((b) => b.setAttribute("aria-label", b.title));
  apply(document);
  new MutationObserver((ms) => {
    for (const m of ms) {
      m.addedNodes.forEach((n) => {
        if (n instanceof HTMLElement) {
          if (n.matches("button[title]:not([aria-label])")) n.setAttribute("aria-label", n.title);
          apply(n);
        }
      });
    }
  }).observe(document.body, { childList: true, subtree: true });
};
