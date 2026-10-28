import { Show, createEffect, createMemo, createSignal } from "solid-js";
import { errorText } from "./appError";
import { specProblem } from "./hostLogic";
import { addHost, setError, showNotice } from "./store";
import { S } from "./strings";
import { R } from "./strings.remote";
import { Button, Field, Input } from "./ui";

/** Name + specification of a new remote host. The checks here are for the user's comfort; the Tauri host validates again. */
export function AddHostForm(props: {
  onAdded?: (id: string) => void;
  onCancel?: () => void;
  /** a suggestion the user clicked (a detected WSL distro or ssh host): fills the name and the specification */
  prefill?: { name: string; spec: string; n: number } | null;
}) {
  const [name, setName] = createSignal("");
  const [spec, setSpec] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const [touched, setTouched] = createSignal(false);
  createEffect(() => {
    const p = props.prefill;
    if (!p) return;
    setName(p.name);
    setSpec(p.spec);
  });
  const specError = createMemo(() => {
    const p = specProblem(spec());
    return p ? R.spec_problem[p] : null;
  });
  const problem = createMemo(() => (name().trim() ? null : R.nameEmpty) ?? specError());
  const submit = async () => {
    setTouched(true);
    if (problem() || busy()) return;
    setBusy(true);
    try {
      const h = await addHost(name(), spec());
      showNotice(R.added(h.name));
      setName("");
      setSpec("");
      setTouched(false);
      props.onAdded?.(h.id);
    } catch (e) {
      setError(R.addFailed(errorText(e)));
    } finally {
      setBusy(false);
    }
  };
  return (
    <form
      class="flex flex-col gap-2"
      aria-label={R.addTitle}
      onSubmit={(e) => {
        e.preventDefault();
        void submit();
      }}
    >
      <Field label={R.name}>
        <Input
          value={name()}
          placeholder={R.namePh}
          autocomplete="off"
          aria-invalid={touched() && !name().trim()}
          onInput={(e) => setName(e.currentTarget.value)}
        />
      </Field>
      <Field label={R.spec} hint={R.specHint}>
        <Input
          value={spec()}
          placeholder={R.specPh}
          autocomplete="off"
          spellcheck={false}
          aria-invalid={touched() && !!specError()}
          aria-describedby="host-spec-error"
          onInput={(e) => setSpec(e.currentTarget.value)}
        />
      </Field>
      <div id="host-spec-error" class="auto-note text-[var(--danger)]" role="alert">
        <Show when={touched() ? problem() : null}>{(m) => m()}</Show>
      </div>
      <div class="m-actions">
        <Show when={props.onCancel}>
          <Button type="button" onClick={() => props.onCancel?.()}>
            {S.cancel}
          </Button>
        </Show>
        <Button type="submit" variant="default" disabled={busy()}>
          {busy() ? R.adding : R.add}
        </Button>
      </div>
    </form>
  );
}
