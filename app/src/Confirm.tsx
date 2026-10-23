import { Show, createSignal } from "solid-js";
import { Modal } from "./Modal";

type Opts = { title?: string; kind?: "warning" | "info" | "error"; okLabel?: string; cancelLabel?: string };
type Pending = { message: string; opts: Opts; resolve: (ok: boolean) => void; /** a third button: its label, and what answering it resolves */ third?: { label: string; resolve: () => void } };

const [pending, setPending] = createSignal<Pending | null>(null);

/** in-app replacement for the native confirm dialog; same call shape as the dialog plugin's `ask` */
export const ask = (message: string, opts: Opts = {}) =>
  new Promise<boolean>((resolve) => {
    pending()?.resolve(false);
    setPending({ message, opts, resolve });
  });

/** three ways out for unsaved text: save it, drop it, or keep editing */
export const askSave = (message: string, title: string) =>
  new Promise<"save" | "discard" | "cancel">((resolve) => {
    pending()?.resolve(false);
    setPending({ message, opts: { title, kind: "warning", okLabel: "Guardar", cancelLabel: "Cancelar" }, resolve: (ok) => resolve(ok ? "save" : "cancel"), third: { label: "Não guardar", resolve: () => resolve("discard") } });
  });

/** discarding unsaved text (a draft, an edited form) */
export const askDiscard = (what: string) => ask(`${what} Descartar as alterações?`, { title: "Descartar alterações", kind: "warning", okLabel: "Descartar", cancelLabel: "Continuar a editar" });

/** a warning or an error is about something that cannot be undone: the safe button gets the focus, not the one that proceeds */
export const focusesCancel = (kind: Opts["kind"]) => kind === "warning" || kind === "error";

const answerThird = () => {
  const p = pending();
  setPending(null);
  p?.third?.resolve();
};

const answer = (ok: boolean) => {
  const p = pending();
  setPending(null);
  p?.resolve(ok);
};

/** mounted once; renders whatever `ask` queued (keyed: a question asked while another is open gets a fresh dialog and focus) */
export function ConfirmHost() {
  return (
    <Show when={pending()} keyed>
      {(p) => (
        <Modal label={p.opts.title ?? "Confirmar"} onClose={() => answer(false)}>
          <div class="m-title" style={{ "font-weight": "600" }}>
            {p.opts.title ?? "Confirmar"}
          </div>
          <div class="auto-note" style={{ "white-space": "pre-wrap" }}>
            {p.message}
          </div>
          <div class="m-actions" style={{ "justify-content": "flex-end" }}>
            <button ref={(el) => focusesCancel(p.opts.kind) && queueMicrotask(() => el.focus())} onClick={() => answer(false)}>{p.opts.cancelLabel ?? "Cancelar"}</button>
            <Show when={p.third}>{(t) => <button class="kind-warn" onClick={answerThird}>{t().label}</button>}</Show>
            <button class="primary" classList={{ "kind-warn": focusesCancel(p.opts.kind) && !p.third }} ref={(el) => !focusesCancel(p.opts.kind) && queueMicrotask(() => el.focus())} onClick={() => answer(true)}>
              {p.opts.okLabel ?? "Sim"}
            </button>
          </div>
        </Modal>
      )}
    </Show>
  );
}
