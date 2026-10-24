import { For, Show, createSignal } from "solid-js";
import {
  applyWorkspaceImport,
  cancelWorkspaceImport,
  clearWorkspaceMap,
  mapWorkspaceFolder,
  pickAndExportWorkspace,
  pickWorkspaceFile,
} from "./api";
import { errorText } from "./settingsLogic";
import { setError, showNotice } from "./store";
import { WS } from "./strings.workspace";
import { Button, Dialog, Field, Input, RadioGroup } from "./ui";
import {
  KINDS,
  SKIPPED_SHOWN,
  WorkspaceConflicts,
  WorkspaceMode,
  WorkspacePreview,
  canImport,
  reasonText,
  sectionLine,
} from "./workspaceLogic";

/** Settings block: save the workspace configuration to a file, or read one (with a preview before anything is written). */
export function WorkspaceSection() {
  const [preview, setPreview] = createSignal<WorkspacePreview | null>(null);
  const [mode, setMode] = createSignal<WorkspaceMode>("merge");
  const [conflicts, setConflicts] = createSignal<WorkspaceConflicts>("keep");
  const [typed, setTyped] = createSignal("");
  const [busy, setBusy] = createSignal(false);

  const guarded = async (work: () => Promise<void>) => {
    setBusy(true);
    try {
      await work();
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  };
  const exportIt = () =>
    guarded(async () => {
      const r = await pickAndExportWorkspace(WS.exportPick);
      if (r === undefined) return; // dialog cancelled
      showNotice(r ? WS.exported(r.path) : WS.noSupport);
    });
  const importIt = () =>
    guarded(async () => {
      const r = await pickWorkspaceFile(WS.importPick);
      if (r === undefined) return;
      if (r === null) return showNotice(WS.noSupport);
      setMode("merge");
      setConflicts("keep");
      setTyped("");
      setPreview(r);
    });
  const close = () => {
    setPreview(null);
    void cancelWorkspaceImport();
  };
  const remap = (old: string) =>
    guarded(async () => {
      const r = await mapWorkspaceFolder(WS.mapPick, old);
      if (r) setPreview(r);
    });
  const clearMap = () =>
    guarded(async () => {
      setPreview(await clearWorkspaceMap());
    });
  const apply = () => {
    const [m, c] = [mode(), conflicts()];
    return guarded(async () => {
      const o = await applyWorkspaceImport(m, c);
      setPreview(null);
      showNotice(WS.doneTitle + WS.done(o) + (o.files.includes("settings.json") ? WS.restartHint : ""));
    });
  };

  return (
    <section class="set-sec">
      <h4>{WS.title}</h4>
      <p class="auto-note">{WS.note}</p>
      <div class="m-actions start">
        <Button disabled={busy()} onClick={() => void exportIt()}>
          {WS.export}
        </Button>
        <Button disabled={busy()} onClick={() => void importIt()}>
          {WS.import}
        </Button>
      </div>
      <Show when={preview()} keyed>
        {(pv) => (
          <Dialog label={WS.dialogTitle} onClose={close} dirty={busy()} closeOnBackdrop={false} class="modal">
            <div class="auto-head">
              <strong>{WS.dialogTitle}</strong>
              <span class="grow" />
            </div>
            <p class="auto-note">{WS.fileLine(pv.file, pv.exported_at)}</p>
            <dl class="set-diag">
              <For each={KINDS}>
                {(k) => (
                  <>
                    <dt>{WS.kinds[k]}</dt>
                    <dd>{sectionLine(pv[k])}</dd>
                  </>
                )}
              </For>
            </dl>
            <For each={KINDS.filter((k) => pv[k].skipped.length > 0)}>
              {(k) => (
                <div class="set-sec" role="group" aria-label={`${WS.kinds[k]}: ${WS.nSkipped(pv[k].skipped.length)}`}>
                  <For each={pv[k].skipped.slice(0, SKIPPED_SHOWN)}>
                    {(s) => (
                      <div class="auto-note">
                        {s.label}: {reasonText(s.reason)}
                        <Show when={k === "projects" && s.reason === "missing" && s.path}>
                          <Button size="sm" variant="outline" disabled={busy()} onClick={() => void remap(s.path!)}>
                            {WS.mapButton}
                          </Button>
                        </Show>
                      </div>
                    )}
                  </For>
                  <Show when={pv[k].skipped.length > SKIPPED_SHOWN}>
                    <div class="auto-note">{WS.moreSkipped(pv[k].skipped.length - SKIPPED_SHOWN)}</div>
                  </Show>
                </div>
              )}
            </For>
            <Show when={pv.map.length > 0}>
              <div class="auto-note">
                {WS.mapped}: {pv.map.map(([from, to]) => `${from} → ${to}`).join("; ")}{" "}
                <Button size="sm" variant="outline" disabled={busy()} onClick={() => void clearMap()}>
                  {WS.mapClear}
                </Button>
              </div>
            </Show>
            <Show when={pv.risky_cards > 0}>
              <div class="text-sm text-destructive" role="alert">
                {WS.risky(pv.risky_cards)}
              </div>
            </Show>
            <RadioGroup
              label={WS.mode}
              value={mode()}
              options={[
                { value: "merge" as WorkspaceMode, label: WS.modeMerge, hint: WS.modeMergeHint },
                { value: "replace" as WorkspaceMode, label: WS.modeReplace, hint: WS.modeReplaceHint },
              ]}
              onChange={(v) => {
                setMode(v);
                setTyped("");
              }}
            />
            <div class="auto-note">{mode() === "merge" ? WS.modeMergeHint : WS.modeReplaceHint}</div>
            <RadioGroup
              label={WS.conflicts}
              value={conflicts()}
              options={[
                { value: "keep" as WorkspaceConflicts, label: WS.conflictsKeep },
                { value: "overwrite" as WorkspaceConflicts, label: WS.conflictsOverwrite },
              ]}
              onChange={setConflicts}
            />
            <Show when={mode() === "replace"}>
              <div class="text-sm text-destructive" role="alert">
                {WS.replaceWarn(pv.projects.local_only)}
              </div>
              <Field label={WS.replaceType(WS.replaceWord)}>
                <Input
                  value={typed()}
                  autocomplete="off"
                  spellcheck={false}
                  onInput={(e) => setTyped(e.currentTarget.value)}
                  onKeyDown={(e) => e.key === "Enter" && canImport(mode(), typed(), busy()) && void apply()}
                />
              </Field>
            </Show>
            <div class="m-actions justify-end">
              <Button onClick={close}>{WS.cancel}</Button>
              <Button
                variant={mode() === "replace" ? "destructive" : "default"}
                class="primary"
                disabled={!canImport(mode(), typed(), busy())}
                onClick={() => void apply()}
              >
                {busy() ? WS.importing : WS.confirm}
              </Button>
            </div>
          </Dialog>
        )}
      </Show>
    </section>
  );
}
