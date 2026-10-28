import { For, Show, createEffect, createSignal, onCleanup, onMount } from "solid-js";
import {
  scanxDefenderApply,
  scanxDefenderRevert,
  scanxDiag,
  scanxStatus,
  type ScanxProc,
  type ScanxStatus,
} from "./api";
import { createLive } from "./live";
import {
  buttons,
  defenderLines,
  folderRows,
  jobMessage,
  perfFocus,
  platformNote,
  procLine,
  setPerfFocus,
  statusError,
} from "./scanxLogic";
import { errorText } from "./settingsLogic";
import { PF } from "./strings.perf";
import { Button, CheckboxField } from "./ui";

type Loaded = ScanxStatus & { loading?: boolean };

/** Settings block: indexer and Defender exclusions of the .axelstra-wt folders. Nothing is applied without the click on Apply. */
export function PerformanceSection() {
  const [st, setSt] = createSignal<Loaded | null>(null);
  const [failed, setFailed] = createSignal("");
  const [consent, setConsent] = createSignal(false);
  const [busy, setBusy] = createSignal(false);
  const [actionError, setActionError] = createSignal("");
  const [procs, setProcs] = createSignal<ScanxProc[] | null>(null);
  const [diagState, setDiagState] = createSignal<"ok" | "n/a">("ok");
  const [diagError, setDiagError] = createSignal("");
  let heading: HTMLHeadingElement | undefined;

  const load = () =>
    scanxStatus().then(
      (s) => {
        setSt(s);
        setFailed("");
      },
      (e) => setFailed(errorText(e)),
    );
  const loadDiag = () =>
    scanxDiag().then(
      (d) => {
        setProcs(d?.procs ?? []);
        setDiagState(d?.state ?? "ok");
        setDiagError("");
      },
      (e) => setDiagError(errorText(e)),
    );

  // the result of an apply/revert arrives as `scanx.changed`; no timer
  const live = createLive(load, ["scanx.changed"], { onFocus: false });
  onMount(() => {
    live.now();
    void loadDiag();
  });
  onCleanup(() => live.stop());

  createEffect(() => {
    if (!perfFocus()) return;
    setPerfFocus(0);
    queueMicrotask(() => {
      heading?.scrollIntoView?.({ block: "start" });
      heading?.focus();
    });
  });

  const start = async (op: "apply" | "revert") => {
    setBusy(true);
    setActionError("");
    try {
      await (op === "apply" ? scanxDefenderApply() : scanxDefenderRevert());
      setConsent(false);
      await load();
    } catch (e) {
      setActionError(errorText(e));
    } finally {
      setBusy(false);
    }
  };

  const view = () => {
    const s = st();
    return s && !s.loading && Array.isArray(s.folders) ? s : null;
  };
  const running = () => busy() || view()?.job.running != null;
  const outcome = () => jobMessage(view()?.job.last);

  return (
    <section class="set-sec perf-sec" aria-labelledby="perf-title">
      <h4 id="perf-title" tabindex="-1" ref={heading}>
        {PF.title}
      </h4>
      <p class="auto-note">{PF.intro}</p>
      <Show
        when={!failed()}
        fallback={
          <div class="set-err" role="alert">
            {failed()}
          </div>
        }
      >
        <Show
          when={view()}
          fallback={
            <p class="auto-note" role="status">
              {st()?.loading ? PF.loadingSlow : PF.loading}
            </p>
          }
        >
          {(s) => (
            <>
              <Show when={s().os === "windows"}>
                <p class="auto-note">{PF.autoIndexer}</p>
              </Show>
              <Show when={platformNote(s())}>{(n) => <p class="auto-note">{n()}</p>}</Show>
              <Show when={statusError(s())}>
                {(e) => (
                  <div class="set-err" role="alert">
                    {e()}
                  </div>
                )}
              </Show>
              <Show when={s().os === "windows"}>
                <ul class="perf-list">
                  <For each={defenderLines(s())}>{(l) => <li class="auto-note">{l}</li>}</For>
                </ul>
              </Show>
              <Show when={s().os === "windows" && !s().folders.length}>
                <p class="auto-note">{PF.noFolders}</p>
              </Show>
              <Show when={s().folders.length}>
                <ul class="perf-list" aria-label={PF.foldersLabel}>
                  <For each={folderRows(s())}>
                    {(r) => (
                      <li class="perf-folder">
                        <code class="perf-path">{r.path}</code>
                        <span class="auto-note">
                          {[r.missing ? PF.folderMissing : r.indexable, r.excluded, r.fs].filter(Boolean).join(" · ")}
                        </span>
                        <Show when={r.devDrive}>
                          <span class="auto-note">{PF.devDrive}</span>
                        </Show>
                        <Show when={r.names.length > 1}>
                          <span class="auto-note">{PF.sharedBy(r.names.join(", "))}</span>
                        </Show>
                      </li>
                    )}
                  </For>
                </ul>
              </Show>
              <Show when={buttons(s(), consent(), busy()).showControls}>
                <p class="auto-note perf-tradeoff">{PF.tradeoff}</p>
                <CheckboxField checked={consent()} onChange={setConsent}>
                  {PF.consent}
                </CheckboxField>
                <p class="auto-note">{PF.uac}</p>
                <div class="set-hooks">
                  <Button disabled={buttons(s(), consent(), busy()).applyDisabled} onClick={() => void start("apply")}>
                    {PF.apply}
                  </Button>
                  <Show when={buttons(s(), consent(), busy()).showRevert}>
                    <Button
                      disabled={buttons(s(), consent(), busy()).revertDisabled}
                      onClick={() => void start("revert")}
                    >
                      {PF.revert}
                    </Button>
                  </Show>
                </div>
                <Show when={buttons(s(), consent(), busy()).showRevert}>
                  <p class="auto-note">{PF.revertNote}</p>
                </Show>
              </Show>
            </>
          )}
        </Show>
      </Show>
      <div class="perf-live" role="status" aria-live="polite">
        <Show when={running()}>
          <span class="auto-note">{PF.running}</span>
        </Show>
        <Show when={!running() && outcome()?.kind === "ok"}>
          <span class="auto-note">{outcome()!.text}</span>
        </Show>
      </div>
      <Show when={actionError() || (!running() && outcome()?.kind === "err")}>
        <div class="set-err" role="alert">
          {actionError() || outcome()!.text}
        </div>
      </Show>
      <h5>{PF.diagTitle}</h5>
      <p class="auto-note">{PF.diagNote}</p>
      <div class="set-hooks">
        <Button onClick={() => void loadDiag()}>{PF.diagRefresh}</Button>
      </div>
      <Show when={diagError()}>
        <div class="set-err" role="alert">
          {diagError()}
        </div>
      </Show>
      <Show when={procs()}>
        {(list) => (
          <Show when={diagState() === "ok"} fallback={<p class="auto-note">{PF.diagNa}</p>}>
            <Show when={list().length} fallback={<p class="auto-note">{PF.diagNone}</p>}>
              <ul class="perf-list" aria-live="polite">
                <For each={list()}>{(p) => <li class="auto-note">{procLine(p)}</li>}</For>
              </ul>
            </Show>
          </Show>
        )}
      </Show>
    </section>
  );
}
