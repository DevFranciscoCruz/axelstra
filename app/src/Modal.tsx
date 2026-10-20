import { JSX, onCleanup, onMount } from "solid-js";

const FOCUSABLE = 'button:not(:disabled), input:not(:disabled), select:not(:disabled), textarea:not(:disabled), [tabindex]:not([tabindex="-1"])';

/** Dialog shell: Esc closes, Tab stays inside, focus returns to where it came from. */
export function Modal(props: { label: string; onClose: () => void; dirty?: boolean; class?: string; backClass?: string; children: JSX.Element }) {
  let box!: HTMLDivElement;
  const from = document.activeElement as HTMLElement | null;
  const onKey = (e: KeyboardEvent) => {
    if (e.key === "Escape") {
      e.stopPropagation();
      props.onClose();
    } else if (e.key === "Tab") {
      const els = [...box.querySelectorAll<HTMLElement>(FOCUSABLE)];
      if (!els.length) return;
      const first = els[0];
      const last = els[els.length - 1];
      if (e.shiftKey && document.activeElement === first) {
        e.preventDefault();
        last.focus();
      } else if (!e.shiftKey && document.activeElement === last) {
        e.preventDefault();
        first.focus();
      }
    }
  };
  onMount(() => {
    window.addEventListener("keydown", onKey, true);
    if (!box.contains(document.activeElement)) box.querySelector<HTMLElement>(FOCUSABLE)?.focus();
  });
  onCleanup(() => {
    window.removeEventListener("keydown", onKey, true);
    from?.focus?.();
  });
  return (
    <div class={props.backClass ?? "modal-back"} onMouseDown={(e) => e.target === e.currentTarget && !props.dirty && props.onClose()}>
      <div ref={box} class={props.class ?? "modal"} role="dialog" aria-modal="true" aria-label={props.label}>
        {props.children}
      </div>
    </div>
  );
}
