import { Show, createSignal } from "solid-js";
import { Modal } from "./Modal";

type Opts = { title?: string; kind?: "warning" | "info" | "error"; okLabel?: string; cancelLabel?: string };
type Pending = { message: string; opts: Opts; resolve: (ok: boolean) => void };

const [pending, setPending] = createSignal<Pending | null>(null);

/** in-app replacement for the native confirm dialog; same call shape as the dialog plugin's `ask` */
export const ask = (message: string, opts: Opts = {}) =>
  new Promise<boolean>((resolve) => {
    pending()?.resolve(false);
    setPending({ message, opts, resolve });
  });

const answer = (ok: boolean) => {
  const p = pending();
  setPending(null);
  p?.resolve(ok);
};

/** mounted once; renders whatever `ask` queued */
export function ConfirmHost() {
  return (
    <Show when={pending()}>
      {(p) => (
        <Modal label={p().opts.title ?? "Confirmar"} onClose={() => answer(false)}>
          <div class="m-title" style={{ "font-weight": "600" }}>
            {p().opts.title ?? "Confirmar"}
          </div>
          <div class="auto-note" style={{ "white-space": "pre-wrap" }}>
            {p().message}
          </div>
          <div class="m-actions" style={{ "justify-content": "flex-end" }}>
            <button onClick={() => answer(false)}>{p().opts.cancelLabel ?? "Cancelar"}</button>
            <button class="primary" ref={(el) => queueMicrotask(() => el.focus())} onClick={() => answer(true)}>
              {p().opts.okLabel ?? "Sim"}
            </button>
          </div>
        </Modal>
      )}
    </Show>
  );
}
