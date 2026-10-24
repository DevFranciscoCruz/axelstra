import { For, Show, createResource, createSignal } from "solid-js";
import { EnvSkip, projectEnvPreview, setProjectSetup } from "./api";
import { coded } from "./msg";
import { Modal } from "./Modal";
import { TriggerSettings } from "./TriggerSettings";
import { projects, refreshProjects, setError } from "./store";
import { Button, Field, Textarea } from "./ui";
import { P } from "./strings.panels";

/** why a file is left out of the next worktree: the daemon's message in the UI language, else its English text */
const skipReason = (s: EnvSkip & { reason_code?: string; reason_params?: Record<string, string> }) =>
  coded(s.reason_code, s.reason_params, s.reason);

/** Per-project settings: the setup command of new worktrees, and a preview of the files `.worktreeinclude` copies. */
export function ProjectSettings(props: { projectId: string; onClose: () => void }) {
  const project = () => projects().find((p) => p.id === props.projectId);
  const [setup, setSetup] = createSignal(project()?.setup ?? "");
  const [saving, setSaving] = createSignal(false);
  const [preview, { refetch }] = createResource(() => props.projectId, projectEnvPreview);
  const dirty = () => setup().trim() !== (project()?.setup ?? "").trim();

  const save = async () => {
    setSaving(true);
    try {
      await setProjectSetup(props.projectId, setup().trim());
      await refreshProjects();
      props.onClose();
    } catch (e) {
      setError(String(e));
    } finally {
      setSaving(false);
    }
  };

  return (
    <Modal label={P.projectSettings.labelOf(project()?.name ?? P.projectSettings.project)} class="modal" dirty={dirty()} onClose={props.onClose}>
      <div class="auto-head">
        <strong>{project()?.name}</strong>
        <span class="grow" />
        <Button onClick={props.onClose}>{P.projectSettings.close}</Button>
      </div>

      <Field label={P.projectSettings.setup}>
        <Textarea
          id="ps-setup"
          rows={4}
          class="font-mono text-xs"
          spellcheck={false}
          placeholder={P.projectSettings.setupPlaceholder}
          value={setup()}
          onInput={(e) => setSetup(e.currentTarget.value)}
          onKeyDown={(e) => {
            // Enter adds a line (one command per line); Ctrl/Cmd+Enter saves
            if (e.key === "Enter" && (e.ctrlKey || e.metaKey) && !saving()) void save();
          }}
        />
      </Field>
      <div class="auto-note">
        {P.projectSettings.setupNote}
      </div>

      <TriggerSettings projectId={props.projectId} />

      <div class="text-xs text-muted-foreground">{P.projectSettings.copied}</div>
      <div class="auto-note">
        {P.projectSettings.includePre}<code>.worktreeinclude</code>{P.projectSettings.includeMid}<code>.env</code>{P.projectSettings.includeOr}<code>.env.local</code>{P.projectSettings.includePost}
      </div>
      <Show when={preview.state === "ready"} fallback={<div class="auto-note">{preview.error ? P.projectSettings.readFailed : P.projectSettings.reading}</div>}>
        <Show
          when={preview()!.has_file}
          fallback={<div class="auto-note">{P.projectSettings.noFilePre}<code>.worktreeinclude</code>{P.projectSettings.noFilePost}</div>}
        >
          <Show when={preview()!.files.length} fallback={<div class="auto-note">{P.projectSettings.noMatch}</div>}>
            <ul class="mono-list" aria-label={P.projectSettings.willCopy}>
              <For each={preview()!.files}>{(f) => <li>{f}</li>}</For>
            </ul>
          </Show>
          <Show when={preview()!.skipped.length}>
            <div class="auto-note">{P.projectSettings.skippedHead}</div>
            <ul class="mono-list" aria-label={P.projectSettings.skippedLabel}>
              <For each={preview()!.skipped}>
                {(s) => (
                  <li>
                    {s.path} <span class="auto-note">— {skipReason(s)}</span>
                  </li>
                )}
              </For>
            </ul>
          </Show>
        </Show>
      </Show>

      <div class="m-actions">
        <Button onClick={() => void refetch()}>{P.projectSettings.refresh}</Button>
        <span class="grow" />
        <Button variant="default" disabled={saving() || !dirty()} onClick={() => void save()}>
          {P.projectSettings.save}
        </Button>
      </div>
    </Modal>
  );
}
