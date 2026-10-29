import { For, Show, createSignal } from "solid-js";
import { memConfig, memResume, type GovernorConfig } from "./api";
import { errorText } from "./appError";
import { Icon } from "./Icon";
import {
  breakdownRows,
  commitPair,
  cpuSplit,
  kernelHigh,
  kernelPoolProblem,
  levelClass,
  parseAmount,
  thresholdProblem,
  type PausedItem,
  type Thresholds,
} from "./memoryLogic";
import { fmtMb } from "./memoryView";
import { fmtFixed, fmtNumber } from "./i18n";
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
        <Show when={breakdownRows(m().breakdown, m().total_mb).length > 0}>
          <div class="sb-head">{G.breakdown.head}</div>
          <Show when={kernelHigh(m().breakdown)}>
            <div class="mem-hint">{G.breakdown.kernelHigh(fmtMb(m().breakdown?.kernel?.total_mb ?? 0))}</div>
          </Show>
          <For each={breakdownRows(m().breakdown, m().total_mb)}>
            {(r) => (
              <>
                <div class="mem-row">
                  <span class="mem-name">{G.breakdown.category[r.key]}</span>
                  <span class="mem-num">{G.breakdown.row(fmtMb(r.mb), G.breakdown.pct(r.pct))}</span>
                </div>
                <div class={r.key.startsWith("kernel") ? "mem-bar mem-kernel" : "mem-bar"}>
                  <i style={{ "--w": `${r.pct}%` }} />
                </div>
              </>
            )}
          </For>
          <Show when={cpuSplit(m().breakdown)}>
            {(c) => (
              <div class="mem-row">
                <span class="mem-name">
                  {G.breakdown.cpu(fmtNumber(Math.round(c().user)), fmtNumber(Math.round(c().kernel)))}
                </span>
              </div>
            )}
          </Show>
          <Show when={commitPair(m().breakdown)}>
            {(c) => (
              <div class="mem-row">
                <span class="mem-name">{G.breakdown.commit(fmtMb(c().used), fmtMb(c().limit))}</span>
              </div>
            )}
          </Show>
          <For each={m().breakdown?.top_cpu ?? []}>
            {(e, i) => (
              <>
                <Show when={i() === 0}>
                  <div class="sb-head">{G.breakdown.topCpuHead}</div>
                </Show>
                <div class="mem-row">
                  <span class="mem-name">{G.breakdown.value(e.name, `${fmtNumber(Math.round(e.value))}%`)}</span>
                </div>
              </>
            )}
          </For>
          <For each={m().breakdown?.top_io ?? []}>
            {(e, i) => (
              <>
                <Show when={i() === 0}>
                  <div class="sb-head">{G.breakdown.topIoHead}</div>
                </Show>
                <div class="mem-row">
                  <span class="mem-name">{G.breakdown.value(e.name, `${fmtNumber(Math.round(e.value))} MB/s`)}</span>
                </div>
              </>
            )}
          </For>
          <Show when={(m().breakdown?.pool_tags?.top ?? []).length > 0}>
            <div class="sb-head">{G.breakdown.poolHead}</div>
            <For each={m().breakdown?.pool_tags?.top ?? []}>
              {(t) => (
                <div class="mem-row">
                  <span class="mem-name">
                    {G.breakdown.pool(t.tag, fmtMb(t.paged_mb), fmtMb(t.nonpaged_mb))}
                    <Show when={t.owner && G.breakdown.poolOwner[t.owner]}>
                      <span class="lim-dim"> · {G.breakdown.poolOwner[t.owner ?? ""]}</span>
                    </Show>
                  </span>
                </div>
              )}
            </For>
            <Show when={m().breakdown?.pool_tags?.sem_etiqueta_mb != null}>
              <div class="mem-row">
                <span class="mem-name">
                  {G.breakdown.poolNoTag(fmtMb(m().breakdown?.pool_tags?.sem_etiqueta_mb ?? 0))}
                </span>
              </div>
            </Show>
            <div class="auto-note">{G.breakdown.poolNote}</div>
          </Show>
          <Show when={m().breakdown?.own}>
            {(o) => (
              <div class="auto-note">
                {G.breakdown.own(
                  fmtNumber(o().spawns),
                  o().spawns_per_min == null ? null : fmtFixed(o().spawns_per_min ?? 0, 1),
                  fmtNumber(o().watches),
                )}
              </div>
            )}
          </Show>
          <Show when={(m().breakdown?.external ?? []).length > 0}>
            <div class="sb-head">{G.breakdown.externalHead}</div>
            <For each={m().breakdown?.external ?? []}>
              {(e) => (
                <div class="mem-row">
                  <span class="mem-name">{G.breakdown.external(e.name, e.count)}</span>
                  <span class="mem-num">{fmtMb(e.rss_mb)}</span>
                </div>
              )}
            </For>
            <div class="auto-note">{G.breakdown.externalNote}</div>
          </Show>
          <Show when={(m().breakdown?.handles?.top ?? []).length > 0}>
            <div class="sb-head">{G.breakdown.handlesHead}</div>
            <For each={m().breakdown?.handles?.top ?? []}>
              {(h) => (
                <div class="mem-row">
                  <span class="mem-name">{G.breakdown.handles(h.name, h.pid, fmtNumber(h.handles))}</span>
                </div>
              )}
            </For>
          </Show>
        </Show>
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
  let kernelDraft: string | undefined;
  const [busy, setBusy] = createSignal(false);
  const saveThresholds = async () => {
    const c = cfg();
    if (!c) return;
    let kernel: number | undefined;
    if (c.kernel_pool_mb !== undefined && kernelDraft !== undefined) {
      const k = parseAmount(kernelDraft);
      if (k === null) return setError(G.settings.notNumber);
      if (kernelPoolProblem(k)) return setError(G.settings.kernelPoolRange);
      kernel = k;
    }
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
      await memConfig(kernel === undefined ? next : { ...next, kernel_pool_mb: kernel });
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
            <Show when={c().kernel_pool_mb !== undefined}>
              <p class="auto-note">{G.settings.kernelPoolNote}</p>
              <Field label={G.settings.kernelPool}>
                <Input
                  type="text"
                  inputmode="numeric"
                  value={String(c().kernel_pool_mb)}
                  spellcheck={false}
                  onInput={(e) => (kernelDraft = e.currentTarget.value)}
                />
              </Field>
            </Show>
            <Button disabled={busy()} onClick={() => void saveThresholds()}>
              {G.settings.save}
            </Button>
          </>
        )}
      </Show>
    </section>
  );
}
