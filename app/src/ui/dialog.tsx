import { Dialog as K } from "@kobalte/core/dialog";
import { JSX, createEffect, onCleanup, onMount } from "solid-js";
import { Portal } from "solid-js/web";
import { menuOpen } from "../ContextMenu";
import { isTopModal, popModal, pushModal, syncShell } from "../modalStack";

const FOCUSABLE = 'button:not(:disabled), input:not(:disabled), select:not(:disabled), textarea:not(:disabled), [tabindex]:not([tabindex="-1"])';

export type DialogProps = {
  label: string;
  onClose: () => void;
  /** unsaved input: a click on the backdrop must not close (and lose) it */
  dirty?: boolean;
  /** false: the backdrop never closes this dialog (a choice that must be made on purpose) */
  closeOnBackdrop?: boolean;
  class?: string;
  backClass?: string;
  children: JSX.Element;
};

/** Modal dialog shell on Kobalte. Real modality: the dialog sits in a portal on <body>, the app shell
 *  (`data-app-shell`) is made `inert` while any dialog is open, and a dialog under another one is inert too. Esc closes the
 *  top dialog only, Tab stays inside it, and focus goes back to where it came from. Kobalte supplies the semantics;
 *  Esc, Tab and the backdrop are handled here so stacked dialogs, menus and Selects keep the layering rules of
 *  `modalStack` (menus live outside the shell, so they still open above a dialog). */
export function Dialog(props: DialogProps) {
  let box: HTMLDivElement | undefined;
  let back: HTMLDivElement | undefined;
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
  // only the top dialog is interactive; the ones under it wait, inert
  createEffect(() => {
    const top = isTopModal(id);
    if (!back) return;
    if (top) back.removeAttribute("inert");
    else back.setAttribute("inert", "");
  });
  onMount(() => {
    window.addEventListener("keydown", onKey, true);
    syncShell();
    // the portal content may be created just after mount: then look for it on the next microtask
    const focusIn = () => {
      if (box && !box.contains(document.activeElement)) box.querySelector<HTMLElement>(FOCUSABLE)?.focus();
    };
    focusIn();
    if (!box) queueMicrotask(focusIn);
  });
  onCleanup(() => {
    window.removeEventListener("keydown", onKey, true);
    popModal(id);
    // the shell (or the dialog underneath) may still be inert for a moment: try now, then once more
    const back = () => from?.isConnected && document.activeElement !== from && from.focus?.();
    back();
    queueMicrotask(back);
  });
  const backdrop = (e: MouseEvent) => {
    if (e.target !== e.currentTarget) return;
    if (props.dirty || props.closeOnBackdrop === false || !isTopModal(id) || menuOpen()) return;
    props.onClose();
  };
  return (
    <Portal>
      <K open modal={false} preventScroll={false}>
        <div ref={back} class={props.backClass ?? "modal-back"} onMouseDown={backdrop}>
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
    </Portal>
  );
}
