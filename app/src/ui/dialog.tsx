import { Dialog as K } from "@kobalte/core/dialog";
import { JSX, onCleanup, onMount } from "solid-js";
import { menuOpen } from "../ContextMenu";
import { isTopModal, popModal, pushModal } from "../modalStack";

const FOCUSABLE = 'button:not(:disabled), input:not(:disabled), select:not(:disabled), textarea:not(:disabled), [tabindex]:not([tabindex="-1"])';

export type DialogProps = { label: string; onClose: () => void; dirty?: boolean; class?: string; backClass?: string; children: JSX.Element };

/** Dialog shell on Kobalte: Esc closes (top dialog only), Tab stays inside, focus returns to where it came from.
 *  Kobalte supplies the dialog semantics; Esc, Tab and the backdrop are handled here so stacked dialogs, menus and
 *  Selects keep the layering rules of `modalStack`. It is not modal on purpose: that would block the menus that
 *  open above a dialog. */
export function Dialog(props: DialogProps) {
  let box: HTMLDivElement | undefined;
  const from = document.activeElement as HTMLElement | null;
  const id = pushModal();
  const onKey = (e: KeyboardEvent) => {
    if (!isTopModal(id) || !box) return;
    // a Select or menu opened inside the dialog is the top layer: it takes Esc and Tab itself
    if (menuOpen()) return;
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
    if (box && !box.contains(document.activeElement)) box.querySelector<HTMLElement>(FOCUSABLE)?.focus();
  });
  onCleanup(() => {
    window.removeEventListener("keydown", onKey, true);
    popModal(id);
    from?.focus?.();
  });
  return (
    <K open modal={false} preventScroll={false}>
      <div class={props.backClass ?? "modal-back"} onMouseDown={(e) => e.target === e.currentTarget && !props.dirty && props.onClose()}>
        <K.Content
          ref={box}
          class={props.class ?? "modal"}
          aria-modal="true"
          aria-label={props.label}
          onEscapeKeyDown={(e) => e.preventDefault()}
          onInteractOutside={(e) => e.preventDefault()}
          onOpenAutoFocus={(e) => e.preventDefault()}
          onCloseAutoFocus={(e) => e.preventDefault()}
        >
          {props.children}
        </K.Content>
      </div>
    </K>
  );
}
