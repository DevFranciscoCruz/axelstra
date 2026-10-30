import { For, JSX, Show, createEffect, createResource, createSignal } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { K } from "./keyboard";
import {
  NotifyMode,
  Settings as SettingsData,
  SettingsPatch,
  budgetCheck,
  ShellChoice,
  Terse,
  BuildSettings,
  DiskSettings,
  JanitorSettings,
  backupExport,
  worktreeSweepFull,
  SweepItem,
  pickAndRestoreBackup,
  copyDiagnostics,
  diagnostics,
  getSettings,
  resetShellCache,
  setSettings,
} from "./api";
import { restartDaemonFlow } from "./daemonActions";
import { BudgetsSection } from "./BudgetsSection";
import { setBudgetRows } from "./budgetStore";
import { AboutSection } from "./About";
import { HooksSection } from "./HooksSection";
import { WorkspaceSection } from "./WorkspaceSection";
import { HostsSection } from "./HostsSection";
import { MemorySection } from "./MemoryPanel";
import { H } from "./strings.hibernate";
import { D } from "./strings.disk";
import { clampInt } from "./diskLogic";
import { idleChoices } from "./hibernateLogic";
import { SettingsPage } from "./SettingsPage";
import { Caps, SectionId, setSection } from "./settingsNav";
import { S } from "./strings";
import {
  DEFAULT_BUILD_JOBS,
  DEFAULT_BUILD_MARGIN_MB,
  DEFAULT_BUILD_MB,
  MAX_BUILD_JOBS,
  MAX_BUILD_MB,
  classifySettingsError,
  diagnosticRows,
  errorText,
  parseBuildNumber,
} from "./settingsLogic";
import { refreshProjects, setError, setSettingsFocus, setShowSettings, settingsFocus, showNotice } from "./store";
import { DEFAULT_FONT, MAX_FONT, MIN_FONT, TermTheme, setTermPrefs, termPrefs } from "./termprefs";
import { APP_THEMES, appTheme, setAppTheme } from "./theme";
import { LOCALES, LOCALE_NAMES, fmtNumber, locale, setLocale } from "./i18n";
import { L } from "./strings.logic";
import { P } from "./strings.panels";
import { ask } from "./Confirm";
import "./termui.css";
import { CATEGORIES, notifyPrefs, setNotifyPref } from "./notifyPrefs";
import { Button, CheckboxField, Field, Input, RadioGroup } from "./ui";

const TERSE: Terse[] = ["off", "short", "tiny"];
const STALL_MINUTES = [0, 5, 10, 15, 30, 60];
const NOTIFY: NotifyMode[] = ["all", "waiting", "off"];
const SHELLS: ShellChoice[] = ["auto", "pwsh", "powershell", "bash", "wsl", "cmd"];
const THEMES: TermTheme[] = ["dark", "light"];

// texts of the backup / diagnostics / accessibility rows
const T = L.settingsRows;

/** A row of exclusive choices (a null value, "auto", is keyed by a sentinel: radio values are strings). */
const NONE = "__none__";
function Choices<T extends string | null>(props: {
  label: string;
  value: T;
  options: { value: T; label: string; hint?: string }[];
  onPick: (v: T) => void;
}) {
  const key = (v: T) => v ?? NONE;
  return (
    <RadioGroup
      label={props.label}
      value={key(props.value)}
      options={props.options.map((o) => ({ value: key(o.value), label: o.label, hint: o.hint }))}
      onChange={(k) => props.onPick(props.options.find((o) => key(o.value) === k)!.value)}
    />
  );
}

const Section = (props: { title: string; note?: string; children: JSX.Element }) => (
  <section class="set-sec">
    <h4>{props.title}</h4>
    <Show when={props.note}>
      <p class="auto-note">{props.note}</p>
    </Show>
    {props.children}
  </section>
);

const AUTO = "auto";
const MANUAL = "manual";

/** A number that is derived by the daemon (null) or fixed by the user: a choice plus, when fixed, a validated field. */
function AutoNumber(props: {
  label: string;
  manualLabel: string;
  hint: string;
  value: number | null;
  max: number;
  fallback: number;
  onSave: (v: number | null) => void;
}) {
  const [bad, setBad] = createSignal(false);
  return (
    <>
      <Choices<string>
        label={props.label}
        value={props.value === null ? AUTO : MANUAL}
        options={[
          { value: AUTO, label: S.auto },
          { value: MANUAL, label: props.manualLabel },
        ]}
        onPick={(v) => {
          setBad(false);
          props.onSave(v === AUTO ? null : (props.value ?? props.fallback));
        }}
      />
      <Show when={props.value !== null}>
        <Field label={props.label}>
          <Input
            type="number"
            min={1}
            max={props.max}
            step={1}
            value={props.value ?? ""}
            aria-invalid={bad()}
            onChange={(e) => {
              const n = parseBuildNumber(e.currentTarget.value, props.max);
              setBad(n === null);
              if (n === null) e.currentTarget.value = String(props.value);
              else props.onSave(n);
            }}
          />
        </Field>
        <Show when={bad()}>
          <div class="auto-note">{S.build.invalid}</div>
        </Show>
      </Show>
      <div class="auto-note">{props.hint}</div>
    </>
  );
}

/** Build environment of the sessions started from now on (`build` in the daemon's settings). */
/** A whole number field saved when it loses focus or Enter is pressed; out of range is pulled back to the range. */
function IntField(props: { label: string; value: number; min: number; max: number; onSave: (n: number) => void }) {
  return (
    <Field label={props.label}>
      <Input
        type="number"
        min={props.min}
        max={props.max}
        step={1}
        value={props.value}
        onChange={(e) => {
          const n = clampInt(e.currentTarget.value, props.min, props.max);
          if (n === null) e.currentTarget.value = String(props.value);
          else if (n !== props.value) props.onSave(n);
        }}
      />
    </Field>
  );
}

/** `disk` and `janitor.artifact_quiet_hours` of settings.json: when builds of idle worktrees go, and what "low space" is. */
function DiskSection(props: {
  disk: DiskSettings;
  janitor: JanitorSettings;
  save: (p: { disk?: DiskSettings; janitor?: JanitorSettings }) => void;
}) {
  const set = (patch: Partial<DiskSettings>) => props.save({ disk: { ...props.disk, ...patch } });
  return (
    <Section title={D.settings.title} note={D.settings.note}>
      <div class="set-fields">
        <IntField
          label={D.settings.quietHours}
          value={props.janitor.artifact_quiet_hours}
          min={1}
          max={720}
          onSave={(artifact_quiet_hours) => props.save({ janitor: { artifact_quiet_hours } })}
        />
      </div>
      <CheckboxField checked={props.disk.enabled} onChange={(enabled) => set({ enabled })}>
        {D.settings.enabled}
      </CheckboxField>
      <Show when={props.disk.enabled}>
        <div class="set-fields">
          <IntField
            label={D.settings.minFreeGb}
            value={props.disk.min_free_gb}
            min={0}
            max={1_000_000}
            onSave={(min_free_gb) => set({ min_free_gb })}
          />
          <IntField
            label={D.settings.minFreePct}
            value={props.disk.min_free_pct}
            min={0}
            max={90}
            onSave={(min_free_pct) => set({ min_free_pct })}
          />
          <IntField
            label={D.settings.marginGb}
            value={props.disk.margin_gb}
            min={0}
            max={1_000_000}
            onSave={(margin_gb) => set({ margin_gb })}
          />
        </div>
      </Show>
    </Section>
  );
}

function BuildSection(props: { cfg: BuildSettings; save: (b: BuildSettings) => void }) {
  const set = (patch: Partial<BuildSettings>) => props.save({ ...props.cfg, ...patch });
  return (
    <Section title={S.build.title} note={S.build.note}>
      <CheckboxField checked={props.cfg.enabled} onChange={(v) => set({ enabled: v })}>
        {S.build.enabled}
      </CheckboxField>
      <Show when={props.cfg.enabled}>
        <AutoNumber
          label={S.build.jobs}
          manualLabel={S.build.jobsManual}
          hint={S.build.jobsHint(fmtNumber(MAX_BUILD_JOBS))}
          value={props.cfg.jobs}
          max={MAX_BUILD_JOBS}
          fallback={DEFAULT_BUILD_JOBS}
          onSave={(jobs) => set({ jobs })}
        />
        <CheckboxField checked={props.cfg.sccache === "auto"} onChange={(v) => set({ sccache: v ? "auto" : "off" })}>
          {S.build.sccache}
        </CheckboxField>
        <div class="auto-note">{S.build.sccacheHint}</div>
        <CheckboxField checked={props.cfg.shared_target} onChange={(v) => set({ shared_target: v })}>
          {S.build.sharedTarget}
        </CheckboxField>
        <div class="auto-note">{S.build.sharedTargetWarn}</div>
        <CheckboxField checked={props.cfg.shared_js_cache} onChange={(v) => set({ shared_js_cache: v })}>
          {S.build.sharedJsCache}
        </CheckboxField>
        <div class="auto-note">{S.build.sharedJsCacheHint}</div>
        <AutoNumber
          label={S.build.budget}
          manualLabel={S.build.budgetManual}
          hint={S.build.budgetHint(fmtNumber(MAX_BUILD_MB))}
          value={props.cfg.lock_budget_mb}
          max={MAX_BUILD_MB}
          fallback={DEFAULT_BUILD_MB}
          onSave={(lock_budget_mb) => set({ lock_budget_mb })}
        />
        <AutoNumber
          label={S.build.margin}
          manualLabel={S.build.budgetManual}
          hint={S.build.marginHint(fmtNumber(MAX_BUILD_MB))}
          value={props.cfg.lock_margin_mb}
          max={MAX_BUILD_MB}
          fallback={DEFAULT_BUILD_MARGIN_MB}
          onSave={(lock_margin_mb) => set({ lock_margin_mb })}
        />
      </Show>
    </Section>
  );
}

/** Daemon version, data paths, lag log and missing tools. Every part is optional: an older build reports less. */
function Diagnostics() {
  const [d, { refetch }] = createResource(async () => {
    const info = await diagnostics().catch((e) => (console.debug("diagnostics", e), null));
    const missing =
      info?.missing_tools ??
      (await invoke<string[]>("missing_tools").catch((e) => (console.debug("missing_tools", e), null)));
    return { info, missing };
  });
  return (
    <Section title={S.diag} note={S.diagNote}>
      <Show when={!d.error && d()} fallback={<div class="auto-note">{S.loading}</div>}>
        {(r) => (
          <>
            <Show when={!r().info}>
              <div class="auto-note">{S.diagUnavailable}</div>
            </Show>
            <dl class="set-diag">
              <For each={diagnosticRows(r().info)}>
                {(row) => (
                  <>
                    <dt>{row.label}</dt>
                    <dd>{row.value}</dd>
                  </>
                )}
              </For>
              <Show when={r().missing}>
                <dt>{S.diagMissing}</dt>
                <dd>{r().missing!.length ? r().missing!.join(", ") : S.diagNoneMissing}</dd>
              </Show>
            </dl>
          </>
        )}
      </Show>
      <div class="m-actions start">
        <Button onClick={() => void refetch()}>{S.diagRun}</Button>
        <Button
          onClick={() =>
            void copyDiagnostics().then(
              () => showNotice(T.diagCopied),
              (e) => setError(errorText(e)),
            )
          }
        >
          {T.copyDiag}
        </Button>
      </div>
    </Section>
  );
}

/** App-wide preferences. Each change is saved at once; agents already open keep their current behaviour. */
export function Settings() {
  const [s, { mutate, refetch }] = createResource(() => getSettings());
  const [restarting, setRestarting] = createSignal(false);
  const save = async (patch: SettingsPatch) => {
    try {
      await setSettings(patch);
      mutate((cur) => (cur ? { ...cur, ...patch } : cur));
      if (patch.shell !== undefined) resetShellCache();
    } catch (e) {
      setError(errorText(e));
    }
  };
  const exportBackup = async () => {
    try {
      const r = await backupExport();
      if (r) showNotice(T.exported + r.path);
      else showNotice(T.noBackup);
    } catch (e) {
      setError(errorText(e));
    }
  };
  const restoreBackup = async () => {
    // restoring replaces settings, projects and sessions: ask before even opening the folder dialog
    if (!(await ask(P.flows.restoreAsk, { title: P.flows.restoreTitle, kind: "warning", okLabel: P.flows.restoreOk })))
      return;
    try {
      const r = await pickAndRestoreBackup(T.restorePath);
      if (r === undefined) return; // dialog cancelled
      showNotice(r ? T.restored : T.noBackup);
    } catch (e) {
      setError(errorText(e));
    }
  };
  const [swept, setSwept] = createSignal<SweepItem[] | null>(null);
  const [closedNote, setClosedNote] = createSignal("");
  const cleanLine = (i: SweepItem) =>
    [i.warning ? T.cleanWarning : i.removed ? T.cleanRemoved : "", i.branch || i.path, i.warning ?? i.why ?? ""]
      .filter(Boolean)
      .join(" · ");
  const cleanNow = async () => {
    // same operation as "Limpar" in the sidebar, which asks first: it closes sessions and removes worktrees of every project
    if (
      !(await ask(P.flows.cleanNowAsk, { title: P.flows.cleanNowTitle, kind: "danger", okLabel: P.flows.cleanNowOk }))
    )
      return;
    try {
      const r = await worktreeSweepFull();
      setSwept(r.swept);
      setClosedNote(
        r.closed_sessions || r.closed_procs
          ? T.cleanClosed(r.closed_sessions ?? 0, r.closed_procs ?? 0, r.freed_bytes ?? 0)
          : "",
      );
      await refreshProjects();
    } catch (e) {
      setError(errorText(e));
    }
  };
  const restartDaemon = () => void restartDaemonFlow(setRestarting, () => void refetch());
  // an older daemon lacks some settings: their pages are not listed. While loading, nothing is hidden (the saved page stays)
  const caps = (): Caps => ({ hibernate: !s() || !!s()?.hibernate, build: !s() || !!s()?.build });
  // the status bar's version chip asks for the About page
  createEffect(() => {
    if (settingsFocus() !== "about") return;
    setSettingsFocus(null);
    setSection("about");
  });
  const page = (id: SectionId, cfg: () => SettingsData) => {
    switch (id) {
      case "general":
        return (
          <>
            <Section title={S.replies} note={S.repliesNote}>
              <Choices
                label={S.replies}
                value={cfg().terse}
                options={TERSE.map((v) => ({ value: v, label: S.terse[v].label, hint: S.terse[v].hint }))}
                onPick={(v) => void save({ terse: v })}
              />
              <div class="auto-note">{S.terse[cfg().terse].hint}</div>
            </Section>
            <Section title={S.pm} note={S.pmNote}>
              <Show when={cfg().installed.length > 0} fallback={<div class="auto-note">{S.pmNone}</div>}>
                <Choices<string | null>
                  label={S.pm}
                  value={cfg().pm}
                  options={[
                    { value: null, label: S.auto },
                    ...cfg().installed.map((m) => ({ value: m as string | null, label: m })),
                  ]}
                  onPick={(v) => void save({ pm: v })}
                />
              </Show>
              <CheckboxField checked={cfg().auto_install} onChange={(v) => void save({ auto_install: v })}>
                {S.autoInstall}
              </CheckboxField>
            </Section>
            <Section title={S.shell} note={S.shellNote}>
              <Choices
                label={S.shell}
                value={cfg().shell ?? "auto"}
                options={SHELLS.map((v) => ({ value: v, label: S.shells[v] }))}
                onPick={(v) => void save({ shell: v })}
              />
            </Section>
            <Section title={S.stall} note={S.stallNote}>
              <Choices
                label={S.stall}
                value={String(cfg().stall_minutes ?? 15)}
                options={[...new Set([...STALL_MINUTES, cfg().stall_minutes ?? 15])]
                  .sort((a, b) => a - b)
                  .map((v) => ({ value: String(v), label: v === 0 ? S.stallOff : S.stallMin(v) }))}
                onPick={(v) => void save({ stall_minutes: Number(v) })}
              />
            </Section>
          </>
        );
      case "language":
        return (
          <Section title={L.lang.title} note={L.lang.note}>
            <Choices
              label={L.lang.title}
              value={locale()}
              options={LOCALES.map((v) => ({ value: v, label: LOCALE_NAMES[v] }))}
              onPick={setLocale}
            />
          </Section>
        );
      case "appearance":
        return (
          <>
            <Section title={S.appearance} note={S.appearanceNote}>
              <Choices
                label={S.appTheme}
                value={appTheme()}
                options={APP_THEMES.map((v) => ({ value: v, label: S.appThemes[v] }))}
                onPick={setAppTheme}
              />
            </Section>
            <Section title={S.terminal} note={K(S.terminalNote)}>
              <Choices
                label={S.termTheme}
                value={termPrefs().theme}
                options={THEMES.map((v) => ({ value: v, label: S.termThemes[v] }))}
                onPick={(v) => setTermPrefs({ theme: v })}
              />
              <div class="set-fields">
                <Field label={S.termFont}>
                  <Input
                    type="text"
                    value={termPrefs().fontFamily}
                    placeholder={DEFAULT_FONT}
                    spellcheck={false}
                    onChange={(e) => setTermPrefs({ fontFamily: e.currentTarget.value })}
                  />
                </Field>
                <Field label={S.termFontSize}>
                  <Input
                    type="number"
                    min={MIN_FONT}
                    max={MAX_FONT}
                    value={termPrefs().fontSize}
                    onChange={(e) => setTermPrefs({ fontSize: Number(e.currentTarget.value) })}
                  />
                </Field>
              </div>
            </Section>
            <Section title={T.a11y} note={T.a11yNote}>
              <CheckboxField checked={termPrefs().screenReader} onChange={(v) => setTermPrefs({ screenReader: v })}>
                {T.screenReader}
              </CheckboxField>
            </Section>
          </>
        );
      case "notifications":
        return (
          <Section title={S.notify} note={S.notifyNote}>
            <Choices
              label={S.notify}
              value={cfg().notify_mode ?? "all"}
              options={NOTIFY.map((v) => ({ value: v, label: S.notifyMode[v].label, hint: S.notifyMode[v].hint }))}
              onPick={(v) => void save({ notify_mode: v })}
            />
            <CheckboxField checked={cfg().notify_sound ?? true} onChange={(v) => void save({ notify_sound: v })}>
              {S.notifySound}
            </CheckboxField>
            <div class="auto-note" id="notify-cats-title">
              {S.notifyCats.title}
            </div>
            <div class="grid gap-1.5" role="group" aria-labelledby="notify-cats-title">
              <For each={CATEGORIES}>
                {(c) => (
                  <CheckboxField checked={notifyPrefs()[c]} onChange={(v) => setNotifyPref(c, v)}>
                    <span class="font-medium">{S.notifyCats[c].label}</span>
                    <span class="auto-note"> · {S.notifyCats[c].hint}</span>
                  </CheckboxField>
                )}
              </For>
              <CheckboxField checked={notifyPrefs().dnd} onChange={(v) => setNotifyPref("dnd", v)}>
                <span class="font-medium">{S.notifyCats.dnd}</span>
                <span class="auto-note"> · {S.notifyCats.dndHint}</span>
              </CheckboxField>
            </div>
            <div class="auto-note">{S.notifyCats.note}</div>
          </Section>
        );
      case "memory":
        return <MemorySection />;
      case "hibernate":
        return (
          <Show when={cfg().hibernate}>
            {(h) => (
              <Section title={H.settings.title} note={H.settings.note}>
                <CheckboxField
                  checked={h().enabled}
                  onChange={(enabled) => void save({ hibernate: { ...h(), enabled } })}
                >
                  {H.settings.enabled}
                </CheckboxField>
                <Show when={h().enabled}>
                  <Choices
                    label={H.settings.idle}
                    value={String(h().idle_minutes)}
                    options={idleChoices(h().idle_minutes).map((v) => ({
                      value: String(v),
                      label: H.settings.idleMin(v),
                    }))}
                    onPick={(v) => void save({ hibernate: { ...h(), idle_minutes: Number(v) } })}
                  />
                </Show>
              </Section>
            )}
          </Show>
        );
      case "build":
        return (
          <Show when={cfg().build}>{(b) => <BuildSection cfg={b()} save={(build) => void save({ build })} />}</Show>
        );
      case "budgets":
        return (
          <BudgetsSection
            budgets={cfg().budgets}
            onSave={(b) => void save({ budgets: b }).then(() => budgetCheck().then(setBudgetRows, () => undefined))}
          />
        );
      case "connections":
        return <HostsSection />;
      case "workspace":
        return <WorkspaceSection />;
      case "hooks":
        return <HooksSection />;
      case "maintenance":
        return (
          <>
            <Section title={T.cleanTitle} note={T.cleanNote}>
              <div class="m-actions start">
                <Button onClick={() => void cleanNow()}>{T.cleanNow}</Button>
              </div>
              <Show when={swept()}>
                {(list) => (
                  <ul class="clean-list">
                    <Show when={closedNote()}>
                      <li>{closedNote()}</li>
                    </Show>
                    <Show when={!list().length && !closedNote()}>
                      <li>{T.cleanNothing}</li>
                    </Show>
                    <For each={list()}>{(i) => <li classList={{ warn: !!i.warning }}>{cleanLine(i)}</li>}</For>
                  </ul>
                )}
              </Show>
            </Section>
            <Show when={cfg().disk && cfg().janitor ? { disk: cfg().disk!, janitor: cfg().janitor! } : undefined}>
              {(d) => <DiskSection disk={d().disk} janitor={d().janitor} save={(p) => void save(p)} />}
            </Show>
            <Section title={T.backup} note={T.backupNote}>
              <div class="m-actions start">
                <Button onClick={() => void exportBackup()}>{T.export}</Button>
                <Button onClick={() => void restoreBackup()}>{T.restore}</Button>
              </div>
            </Section>
          </>
        );
      case "diagnostics":
        return <Diagnostics />;
      case "about":
        return <AboutSection />;
    }
  };
  return (
    <SettingsPage
      caps={caps()}
      onClose={() => setShowSettings(false)}
      page={(id) => (
        <Show
          when={s.state !== "errored"}
          fallback={
            <div class="set-error">
              <div class="auto-note">
                {classifySettingsError(s.error) === "old-daemon" ? S.oldDaemon : S.loadFailed + errorText(s.error)}
              </div>
              <div class="m-actions">
                <Show when={classifySettingsError(s.error) === "old-daemon"}>
                  <Button variant="default" disabled={restarting()} onClick={restartDaemon}>
                    {restarting() ? S.restartingDaemon : S.restartDaemon}
                  </Button>
                </Show>
                <Button disabled={restarting()} onClick={() => void refetch()}>
                  {S.retry}
                </Button>
              </div>
            </div>
          }
        >
          <Show when={s()} fallback={<div class="auto-note">{S.loading}</div>}>
            {(cfg) => page(id, cfg)}
          </Show>
        </Show>
      )}
    />
  );
}
