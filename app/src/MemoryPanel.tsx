import { For, Show, createSignal } from "solid-js";
import { memConfig, memResume, type GovernorConfig } from "./api";
import { errorText } from "./appError";
import { Icon } from "./Icon";
import { levelClass, parseAmount, thresholdProblem, type PausedItem, type Thresholds } from "./memoryLogic";
import { fmtMb } from "./memoryView";
import { setSection } from "./settingsNav";
import { memory, refreshMemory, sessions, setError, setShowSettings, showNotice, tabLabel } from "./store";
import { G } from "./strings.memory";
import { ago } from "./time";
import { Button, CheckboxField, Field, Input } from "./ui";

/** Name of a session for the lists; a session that is gone is just its number. */
const nameOf = (id: number | null): string => {
  if (id === null) return "";
  const s = sessions().find((x) => x.id === id);
  return s ? tabLabel(s) : G.panel.session(id);
};

const pausedName = (p: PausedItem): string =>
  p.kind === "build" ? G.panel.build(p.name ?? nameOf(p.session)) : nameOf(p.session);

/** Runs a governor action, shows the daemon's refusal in the language of the app, and reads the status again either way. */
const act = async (f: () => Promise<unknown>) => {
  try {
    await f();
  } catch (e) {
    setError(errorText(e));
  }
  await refreshMemory();
};

/** Body of the status bar popover: level and free memory, the sessions that use most, what is paused and a way to resume it. */
export function MemoryPanelBody(props: { close: () => void }) {
  const m = memory;
  return (
    <div class="mem-panel" role="group" aria-label={G.panel.title}>
      <Show when={m().loaded} fallback={<div class="sb-empty">{G.panel.oldDaemon}</div>}>
        <div class={`mem-sum ${levelClass(m().level)}`}>
          {G.panel.summary(G.level[m().level], G.size(m().free_mb), G.size(m().total_mb))}
        </div>
        <div class="sb-head">{G.panel.top}</div>
        <For each={m().top.slice(0, 5)} fallback={<div class="sb-empty">{G.panel.topNone}</div>}>
          {(t) => (
            <div class="mem-row">
              <span class="mem-name">{nameOf(t.session)}</span>
              <span class="mem-num">{G.panel.rss(fmtMb(t.rss_mb))}</span>
            </div>
          )}
        </For>
        <div class="sb-head">
          {G.panel.pausedHead} <span class="lim-dim">({m().paused.length})</span>
        </div>
        <For each={m().paused} fallback={<div class="sb-empty">{G.panel.pausedNone}</div>}>
          {(p) => (
            <div class="mem-row">
              <span class="mem-name">
                {pausedName(p)}{" "}
                <span class="lim-dim">{p.reason === "memory" ? G.panel.pausedMemory : G.panel.pausedManual}</span>
              </span>
              <span class="mem-num">{ago((Date.now() - p.since) / 1000)}</span>
              <Show when={p.session !== null}>
                <Button
                  variant="chip"
                  size="chip"
                  aria-label={G.panel.resumeOne(pausedName(p))}
                  onClick={() => void act(() => memResume(p.session!))}
                >
                  <Icon name="play" size={12} />
                  {G.panel.resume}
                </Button>
              </Show>
            </div>
          )}
        </For>
        <div class="mem-row">
          <Show when={m().paused.length > 0}>
            <Button variant="outline" size="sm" onClick={() => void act(() => memResume("all"))}>
              {G.panel.resumeAll}
            </Button>
          </Show>
          <Button
            variant="outline"
            size="sm"
            onClick={() => {
              props.close();
              setSection("memory");
              setShowSettings(true);
            }}
          >
            {G.panel.configure}
          </Button>
        </div>
      </Show>
    </div>
  );
}

type ThresholdKey = keyof Thresholds;
const THRESHOLD_FIELDS: { key: ThresholdKey; label: () => string }[] = [
  { key: "high_free_pct", label: () => G.settings.highPct },
  { key: "crit_free_pct", label: () => G.settings.critPct },
  { key: "high_free_mb", label: () => G.settings.highMb },
  { key: "crit_free_mb", label: () => G.settings.critMb },
];

/** Settings block of the resource governor: the four switches and the free-memory thresholds. */
export function MemorySection() {
  const cfg = (): GovernorConfig | null => memory().config;
  const patch = (p: Partial<GovernorConfig>) => void act(() => memConfig(p));
  const draft: Partial<Record<ThresholdKey, string>> = {};
  const [busy, setBusy] = createSignal(false);
  const saveThresholds = async () => {
    const c = cfg();
    if (!c) return;
    const next = {} as Thresholds;
    for (const f of THRESHOLD_FIELDS) {
      const typed = draft[f.key];
      const n = typed === undefined ? c[f.key] : parseAmount(typed);
      if (n === null) return setError(G.settings.notNumber);
      next[f.key] = n;
    }
    const problem = thresholdProblem(next);
    if (problem) return setError(G.settings.problem[problem]);
    setBusy(true);
    try {
      await memConfig(next);
      showNotice(G.settings.saved);
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
    await refreshMemory();
  };
  return (
    <section class="set-sec">
      <h4>{G.settings.title}</h4>
      <p class="auto-note">{G.settings.note}</p>
      <Show when={cfg()} fallback={<div class="auto-note">{G.panel.oldDaemon}</div>}>
        {(c) => (
          <>
            <CheckboxField checked={c().enabled} onChange={(v) => patch({ enabled: v })}>
              {G.settings.enabled}
            </CheckboxField>
            <CheckboxField checked={c().admission} onChange={(v) => patch({ admission: v })}>
              {G.settings.admission}
            </CheckboxField>
            <CheckboxField checked={c().auto_pause} onChange={(v) => patch({ auto_pause: v })}>
              {G.settings.autoPause}
            </CheckboxField>
            <CheckboxField checked={c().warn_only} onChange={(v) => patch({ warn_only: v })}>
              {G.settings.warnOnly}
            </CheckboxField>
            <div class="auto-note">{G.settings.warnOnlyHint}</div>
            <h5 class="bud-sub">{G.settings.thresholds}</h5>
            <p class="auto-note">{G.settings.thresholdsNote}</p>
            <div class="mem-set-grid">
              <For each={THRESHOLD_FIELDS}>
                {(f) => (
                  <Field label={f.label()}>
                    <Input
                      type="text"
                      inputmode="numeric"
                      value={String(c()[f.key])}
                      spellcheck={false}
                      onInput={(e) => (draft[f.key] = e.currentTarget.value)}
                    />
                  </Field>
                )}
              </For>
            </div>
            <Button disabled={busy()} onClick={() => void saveThresholds()}>
              {G.settings.save}
            </Button>
          </>
        )}
      </Show>
    </section>
  );
}
