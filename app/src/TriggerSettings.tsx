import { For, Show, createResource, createSignal } from "solid-js";
import { triggersGet, triggersPreview, triggersSet, type TriggerState } from "./api";
import { ask } from "./Confirm";
import { setError } from "./store";
import { P } from "./strings.panels";
import { cooldownPatch, firingKind, firingText, perHourPatch, toggleTrigger } from "./triggerLogic";
import { CheckboxField, Field, Input } from "./ui";

/** The CI-fix trigger of one project: on/off (the first time behind a confirmation that lists what will happen), its limits and what it did lately. */
export function TriggerSettings(props: { projectId: string }) {
  const [state, { mutate }] = createResource(() => props.projectId, triggersGet);
  const [busy, setBusy] = createSignal(false);

  const apply = async (patch: Parameters<typeof triggersSet>[1]) => {
    setBusy(true);
    try {
      mutate(await triggersSet(props.projectId, patch));
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const toggle = async (on: boolean, s: TriggerState) => {
    setBusy(true);
    try {
      const next = await toggleTrigger(props.projectId, on, s, { preview: triggersPreview, ask, set: triggersSet });
      if (next) mutate(next);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <section class="flex flex-col gap-2" aria-label={P.triggers.title}>
      <Show when={state.state !== "pending"} fallback={<div class="auto-note">{P.triggers.loading}</div>}>
        <Show when={state()} fallback={<div class="auto-note">{P.triggers.oldDaemon}</div>}>
          {(s) => (
            <>
              <CheckboxField checked={s().ci_fix} onChange={(v) => void (busy() ? undefined : toggle(v, s()))}>
                {P.triggers.toggle}
              </CheckboxField>
              <div class="auto-note">{P.triggers.explain}</div>
              <div class="flex gap-3">
                <Field label={P.triggers.cooldown}>
                  <Input
                    type="number"
                    min={s().limits.cooldown_range[0] / 60}
                    max={s().limits.cooldown_range[1] / 60}
                    value={Math.round(s().limits.cooldown_secs / 60)}
                    disabled={busy()}
                    onChange={(e) => {
                      const n = cooldownPatch(e.currentTarget.value, s().limits);
                      if (n != null) void apply({ cooldown_secs: n });
                    }}
                  />
                </Field>
                <Field label={P.triggers.perHour}>
                  <Input
                    type="number"
                    min={s().limits.max_per_hour_range[0]}
                    max={s().limits.max_per_hour_range[1]}
                    value={s().limits.max_per_hour}
                    disabled={busy()}
                    onChange={(e) => {
                      const n = perHourPatch(e.currentTarget.value, s().limits);
                      if (n != null) void apply({ max_per_hour: n });
                    }}
                  />
                </Field>
              </div>
              <div class="auto-note">{P.triggers.limitsNote(s().limits.poll_secs)}</div>
              <div class="text-xs text-muted-foreground">{P.triggers.recent}</div>
              <Show when={s().history.length} fallback={<div class="auto-note">{P.triggers.none}</div>}>
                <ul class="m-0 list-none p-0 text-xs" aria-label={P.triggers.recent}>
                  <For each={[...s().history].reverse()}>
                    {(f) => (
                      <li>
                        <span class="text-muted-foreground">[{firingKind(f)}]</span> {firingText(f)}
                      </li>
                    )}
                  </For>
                </ul>
              </Show>
            </>
          )}
        </Show>
      </Show>
    </section>
  );
}
