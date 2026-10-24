import { Show, createSignal } from "solid-js";
import { Modal } from "./Modal";
import { Button, Input } from "./ui";

type Opts = { title?: string; kind?: "danger" | "warning" | "info" | "error"; okLabel?: string; cancelLabel?: string;
  /** the OK button stays off until this exact text is typed (removing work that is not saved anywhere else) */
  typeToConfirm?: string };
type Pending = { message: string; opts: Opts; resolve: (ok: boolean) => void; /** a third button: its label, and what answering it resolves */ third?: { label: string; resolve: () => void } };

const [pending, setPending] = createSignal<Pending | null>(null);
const [typed, setTyped] = createSignal("");

/** in-app replacement for the native confirm dialog; same call shape as the dialog plugin's `ask` */
export const ask = (message: string, opts: Opts = {}) =>
  new Promise<boolean>((resolve) => {
    pending()?.resolve(false);
    setTyped("");
    setPending({ message, opts, resolve });
  });

/** the typed text matches, ignoring case and surrounding spaces */
export const typedMatches = (expected: string, typed: string) => typed.trim().toLowerCase() === expected.trim().toLowerCase() && expected.trim() !== "";

/** three ways out for unsaved text: save it, drop it, or keep editing */
export const askSave = (message: string, title: string) =>
  new Promise<"save" | "discard" | "cancel">((resolve) => {
    pending()?.resolve(false);
    setPending({ message, opts: { title, kind: "warning", okLabel: "Guardar", cancelLabel: "Cancelar" }, resolve: (ok) => resolve(ok ? "save" : "cancel"), third: { label: "Não guardar", resolve: () => resolve("discard") } });
  });

/** discarding unsaved text (a draft, an edited form) */
export const askDiscard = (what: string) => ask(`${what} Descartar as alterações?`, { title: "Descartar alterações", kind: "warning", okLabel: "Descartar", cancelLabel: "Continuar a editar" });

/** a danger or an error is about something that cannot be undone, a warning about something that is merely consequential (push, merge): both focus the safe button, not the one that proceeds */
export const focusesCancel = (kind: Opts["kind"]) => kind === "danger" || kind === "warning" || kind === "error";

/** only irreversible actions get the red confirm button */
export const isDanger = (kind: Opts["kind"]) => kind === "danger" || kind === "error";

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
          <Show when={p.opts.typeToConfirm}>
            {(want) => (
              <Input
                aria-label={`Escreva "${want()}" para confirmar`}
                placeholder={want()}
                value={typed()}
                onInput={(e) => setTyped(e.currentTarget.value)}
                onKeyDown={(e) => e.key === "Enter" && typedMatches(want(), typed()) && answer(true)}
              />
            )}
          </Show>
          <div class="m-actions" style={{ "justify-content": "flex-end" }}>
            <Button ref={(el: HTMLButtonElement) => focusesCancel(p.opts.kind) && queueMicrotask(() => el.focus())} onClick={() => answer(false)}>{p.opts.cancelLabel ?? "Cancelar"}</Button>
            <Show when={p.third}>{(t) => <Button variant="destructive" onClick={answerThird}>{t().label}</Button>}</Show>
            <Button variant={isDanger(p.opts.kind) && !p.third ? "destructive" : "default"} class={isDanger(p.opts.kind) && !p.third ? "primary kind-warn" : "primary"} ref={(el: HTMLButtonElement) => !focusesCancel(p.opts.kind) && queueMicrotask(() => el.focus())} disabled={!!p.opts.typeToConfirm && !typedMatches(p.opts.typeToConfirm, typed())} onClick={() => answer(true)}>
              {p.opts.okLabel ?? "Sim"}
            </Button>
          </div>
        </Modal>
      )}
    </Show>
  );
}
