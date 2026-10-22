import { JSX, onCleanup, onMount } from "solid-js";
import { isTopModal, popModal, pushModal } from "./modalStack";

const FOCUSABLE = 'button:not(:disabled), input:not(:disabled), select:not(:disabled), textarea:not(:disabled), [tabindex]:not([tabindex="-1"])';

/** Dialog shell: Esc closes (top modal only), Tab stays inside, focus returns to where it came from. */
export function Modal(props: { label: string; onClose: () => void; dirty?: boolean; class?: string; backClass?: string; children: JSX.Element }) {
  let box!: HTMLDivElement;
  const from = document.activeElement as HTMLElement | null;
  // stacked modals (a Confirm over Settings): only the top one handles keys
  const id = pushModal();
  const onKey = (e: KeyboardEvent) => {
    if (!isTopModal(id)) return;
    if (e.key === "Escape") {
      e.stopImmediatePropagation();
      props.onClose();
    } else if (e.key === "Tab") {
      const els = [...box.querySelectorAll<HTMLElement>(FOCUSABLE)];
      if (!els.length) {
        e.preventDefault();
        return;
      }
      const first = els[0];
      const last = els[els.length - 1];
      if (!box.contains(document.activeElement)) {
        e.preventDefault();
        (e.shiftKey ? last : first).focus();
      } else if (e.shiftKey && document.activeElement === first) {
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
    popModal(id);
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
